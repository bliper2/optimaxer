#!/usr/bin/env python3
"""Optimaxer for Manjaro / Arch Linux: tweaks with undo, app installer, cleaner, services, startup, debloat, DNS and tools.

Standard library only. Run `optimaxer` for the interactive menu or `optimaxer --help` for the non-interactive commands.
Everything that changes the system records what it changed in /var/lib/optimaxer/state.json so it can be undone.
Use --dry-run to see every command and file change without making any.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request

__version__ = "2.3.2"
REPO = "bliper2/optimaxer"
STATE_FILE = "/var/lib/optimaxer/state.json"
BACKUP_DIR = "/var/lib/optimaxer/backups"
LOG_FILE = "/var/log/optimaxer.log"

# ======================================================================================================== context

class Ctx:
    root = ""          # prefix for every path (tests use a temp dir)
    dry_run = False
    assume_yes = False
    runner = None      # test hook: callable(cmd, user, write, input) -> (rc, out)
    user = None
    home = None
    quiet = False
    log_hook = None    # the GUI sets this to receive log lines


CTX = Ctx()


def P(path: str) -> str:
    """Map an absolute system path under the optional test root."""
    return os.path.join(CTX.root, path.lstrip("/")) if CTX.root else path


def log(msg: str, level: str = "INFO") -> None:
    line = f"[{time.strftime('%H:%M:%S')}] {level:<5} {msg}"
    if CTX.log_hook:
        CTX.log_hook(line, level)
    elif not CTX.quiet:
        print(line)
    if not CTX.dry_run and not CTX.root:
        try:
            with open(LOG_FILE, "a", encoding="utf-8") as fh:
                fh.write(line + "\n")
        except OSError:
            pass


def real_user() -> str:
    if CTX.user:
        return CTX.user
    return os.environ.get("SUDO_USER") or os.environ.get("USER") or "root"


def real_home() -> str:
    if CTX.home:
        return CTX.home
    try:
        import pwd
        return pwd.getpwnam(real_user()).pw_dir
    except Exception:
        return os.path.expanduser("~")


def run(cmd, write: bool = False, as_user: bool = False, input: str | None = None, timeout: int | None = None):
    """Run a command, returning (returncode, combined output). Writes are skipped in dry-run mode."""
    if isinstance(cmd, str):
        cmd = ["sh", "-c", cmd]
    if write and CTX.dry_run:
        log("[dry-run] " + " ".join(cmd), "DRY")
        return 0, ""
    if CTX.runner:
        return CTX.runner(cmd, real_user() if as_user else None, write, input)
    if as_user and hasattr(os, "geteuid") and os.geteuid() == 0 and real_user() != "root":
        cmd = ["sudo", "-u", real_user(), "-H"] + cmd
    try:
        p = subprocess.run(cmd, input=input, capture_output=True, text=True, timeout=timeout)
        return p.returncode, (p.stdout or "") + (p.stderr or "")
    except FileNotFoundError:
        return 127, f"{cmd[0]}: command not found"
    except subprocess.TimeoutExpired:
        return 124, "timed out"


def have(cmd: str) -> bool:
    if CTX.runner:
        return CTX.runner(["which", cmd], None, False, None)[0] == 0
    return shutil.which(cmd) is not None


# ======================================================================================================== files & state

def read_text(path: str):
    try:
        with open(P(path), "r", encoding="utf-8") as fh:
            return fh.read()
    except (FileNotFoundError, NotADirectoryError):
        return None


def write_text(path: str, content: str) -> None:
    if CTX.dry_run:
        log(f"[dry-run] write {path} ({len(content)} bytes)", "DRY")
        return
    full = P(path)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w", encoding="utf-8") as fh:
        fh.write(content)


def remove_file(path: str) -> None:
    if CTX.dry_run:
        log(f"[dry-run] remove {path}", "DRY")
        return
    try:
        os.remove(P(path))
    except FileNotFoundError:
        pass


def load_state() -> dict:
    raw = read_text(STATE_FILE)
    if not raw:
        return {}
    try:
        return json.loads(raw)
    except ValueError:
        return {}


def save_state(state: dict) -> None:
    if CTX.dry_run:
        return
    write_text(STATE_FILE, json.dumps(state, indent=2))


def parse_os_release(text: str | None) -> dict:
    out = {}
    for line in (text or "").splitlines():
        if "=" in line and not line.startswith("#"):
            k, v = line.split("=", 1)
            out[k.strip()] = v.strip().strip('"')
    return out


def distro() -> dict:
    return parse_os_release(read_text("/etc/os-release"))


def is_arch_family() -> bool:
    d = distro()
    ids = {d.get("ID", "")} | set(d.get("ID_LIKE", "").split())
    return bool(ids & {"manjaro", "arch", "endeavouros", "garuda", "cachyos", "artix"})


def fmt_size(n: float) -> str:
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if n < 1024 or unit == "TB":
            return f"{n:.0f} {unit}" if unit == "B" else f"{n:.1f} {unit}"
        n /= 1024
    return f"{n:.1f} TB"


def dir_size(path: str) -> int:
    total = 0
    full = P(path)
    if os.path.isfile(full):
        try:
            return os.path.getsize(full)
        except OSError:
            return 0
    for base, _dirs, files in os.walk(full, onerror=lambda e: None):
        for f in files:
            try:
                total += os.lstat(os.path.join(base, f)).st_size
            except OSError:
                pass
    return total


# ======================================================================================================== line edits

class LineEdit:
    """Set `key = value` (or a bare flag when value is None) in a config file, remembering the original line."""

    def __init__(self, path: str, key: str, value: str | None = None, section: str | None = None, sep: str = " = "):
        self.path, self.key, self.value, self.section, self.sep = path, key, value, section, sep

    @property
    def desired(self) -> str:
        return self.key if self.value is None else f"{self.key}{self.sep}{self.value}"

    def _active(self, line: str) -> bool:
        return re.match(rf"^\s*{re.escape(self.key)}\s*(=|$)", line) is not None

    def _commented(self, line: str) -> bool:
        return re.match(rf"^\s*#\s*{re.escape(self.key)}\s*(=|$)", line) is not None

    @staticmethod
    def _norm(line: str) -> str:
        return re.sub(r"\s*=\s*", "=", line.strip())

    def is_applied(self) -> bool:
        text = read_text(self.path)
        if text is None:
            return False
        return any(self._active(l) and self._norm(l) == self._norm(self.desired) for l in text.split("\n"))

    def apply(self, text: str):
        """Return (new_text, original_line_or_None). original_line is None when a line was appended."""
        lines = text.split("\n")
        for i, l in enumerate(lines):
            if self._active(l):
                if self._norm(l) == self._norm(self.desired):
                    return text, l
                lines[i] = self.desired
                return "\n".join(lines), l
        for i, l in enumerate(lines):
            if self._commented(l):
                lines[i] = self.desired
                return "\n".join(lines), l
        if self.section:
            for i, l in enumerate(lines):
                if l.strip().lower() == f"[{self.section.lower()}]":
                    lines.insert(i + 1, self.desired)
                    return "\n".join(lines), None
        if lines and lines[-1] == "":
            lines.insert(len(lines) - 1, self.desired)
        else:
            lines.append(self.desired)
        return "\n".join(lines), None

    def revert(self, text: str, original: str | None) -> str:
        lines = text.split("\n")
        for i, l in enumerate(lines):
            if self._active(l) and self._norm(l) == self._norm(self.desired):
                if original is None:
                    del lines[i]
                else:
                    lines[i] = original
                break
        return "\n".join(lines)


# ======================================================================================================== systemd / packages helpers

def unit_exists(unit: str) -> bool:
    rc, out = run(["systemctl", "list-unit-files", unit, "--no-legend"])
    return rc == 0 and unit.split(".")[0] in out


def unit_enabled(unit: str) -> bool:
    rc, out = run(["systemctl", "is-enabled", unit])
    return out.strip() in ("enabled", "enabled-runtime")


def unit_active(unit: str) -> bool:
    return run(["systemctl", "is-active", unit])[0] == 0


def pkg_installed(pkg: str) -> bool:
    return run(["pacman", "-Q", pkg])[0] == 0


def flatpak_installed(app: str) -> bool:
    return have("flatpak") and run(["flatpak", "info", app])[0] == 0


# ======================================================================================================== tweaks

class Tweak:
    def __init__(self, id, name, desc, cat, risk="safe", tags=(), files=None, edits=None, pkgs=None, enable=None, disable=None,
                 post=None, undo_post=None, cmds=None, undo_cmds=None, test_cmd=None, needs=None):
        self.id, self.name, self.desc, self.cat, self.risk = id, name, desc, cat, risk
        self.tags = set(tags)
        self.files = files or {}          # path -> content
        self.edits = edits or []          # [LineEdit]
        self.pkgs = pkgs or []            # pacman packages installed first (kept on undo)
        self.enable = enable or []        # units to enable --now
        self.disable = disable or []      # units to disable --now
        self.post = post or []            # commands after applying
        self.undo_post = undo_post or []  # commands after undoing
        self.cmds = cmds or []            # extra commands during apply
        self.undo_cmds = undo_cmds or []
        self.test_cmd = test_cmd          # command that exits 0 when applied (extra check)
        self.needs = needs or []          # files or commands that must exist, else the tweak is unavailable

    def available(self) -> bool:
        for n in self.needs:
            if n.startswith("/"):
                if read_text(n) is None:
                    return False
            elif not have(n):
                return False
        return True

    def is_applied(self) -> bool:
        for path, content in self.files.items():
            if read_text(path) != content:
                return False
        for e in self.edits:
            if not e.is_applied():
                return False
        for u in self.enable:
            if not unit_enabled(u):
                return False
        for u in self.disable:
            if unit_enabled(u):
                return False
        for p in self.pkgs:
            if not pkg_installed(p):
                return False
        if self.test_cmd and run(self.test_cmd)[0] != 0:
            return False
        return True

    def apply(self) -> bool:
        state = load_state()
        already = self.id in state and self.is_applied()
        snap = {"files": [], "edits": [], "units": [], "pkgs_added": [], "time": time.strftime("%Y-%m-%dT%H:%M:%S")}
        if already:
            snap = state[self.id]
        log(f"Apply: {self.name}")
        missing = [p for p in self.pkgs if not pkg_installed(p)]
        if missing:
            log(f"  installing packages: {' '.join(missing)}")
            rc, out = run(["pacman", "-S", "--needed", "--noconfirm"] + missing, write=True)
            if rc != 0:
                log(f"  package install failed: {out.strip().splitlines()[-1] if out.strip() else rc}", "WARN")
                return False
            if not already:
                snap["pkgs_added"] = missing
        for path, content in self.files.items():
            if not already:
                prev = read_text(path)
                snap["files"].append({"path": path, "existed": prev is not None, "content": prev})
            write_text(path, content)
        for e in self.edits:
            text = read_text(e.path)
            if text is None:
                log(f"  {e.path} not found, skipped", "WARN")
                continue
            new, orig = e.apply(text)
            if not already:
                snap["edits"].append({"path": e.path, "key": e.key, "value": e.value, "section": e.section, "sep": e.sep,
                                      "orig": orig, "changed": new != text})
            if new != text:
                if not CTX.dry_run:
                    os.makedirs(P(BACKUP_DIR), exist_ok=True)
                    shutil.copy2(P(e.path), os.path.join(P(BACKUP_DIR), os.path.basename(e.path) + "." + time.strftime("%Y%m%d-%H%M%S") + ".bak"))
                write_text(e.path, new)
        for u in self.enable + self.disable:
            if not already:
                snap["units"].append({"unit": u, "enabled": unit_enabled(u), "active": unit_active(u)})
        for c in self.cmds:
            run(c, write=True)
        for u in self.enable:
            run(["systemctl", "enable", "--now", u], write=True)
        for u in self.disable:
            run(["systemctl", "disable", "--now", u], write=True)
        for c in self.post:
            rc, out = run(c, write=True)
            if rc != 0:
                log(f"  command failed ({' '.join(c) if isinstance(c, list) else c}): {out.strip()[-120:]}", "WARN")
        state[self.id] = snap
        save_state(state)
        ok = CTX.dry_run or self.is_applied()
        log(("  verified" if ok else "  NOT verified: check the messages above"), "OK" if ok else "WARN")
        return ok

    def undo(self) -> bool:
        state = load_state()
        snap = state.get(self.id)
        log(f"Undo: {self.name}")
        if not snap:
            log("  no saved original values for this tweak (was it applied by Optimaxer?)", "WARN")
            return False
        for u in snap.get("units", []):
            if u["unit"] in self.enable and not u["enabled"]:
                run(["systemctl", "disable", "--now", u["unit"]], write=True)
            if u["unit"] in self.disable and u["enabled"]:
                run(["systemctl", "enable"] + (["--now"] if u["active"] else []) + [u["unit"]], write=True)
        for f in snap.get("files", []):
            if f["existed"]:
                write_text(f["path"], f["content"])
            else:
                remove_file(f["path"])
        for ed in snap.get("edits", []):
            if not ed.get("changed"):
                continue
            text = read_text(ed["path"])
            if text is None:
                continue
            e = LineEdit(ed["path"], ed["key"], ed["value"], ed.get("section"), ed.get("sep", " = "))
            write_text(ed["path"], e.revert(text, ed["orig"]))
        for c in self.undo_cmds + self.undo_post + self.post_for_undo():
            run(c, write=True)
        if snap.get("pkgs_added"):
            log(f"  kept packages installed by this tweak: {' '.join(snap['pkgs_added'])} (remove with pacman -Rns if unwanted)")
        state.pop(self.id, None)
        save_state(state)
        ok = CTX.dry_run or not self.is_applied()
        log(("  reverted" if ok else "  still looks applied"), "OK" if ok else "WARN")
        return ok

    def post_for_undo(self):
        # configuration reloads that make removed files take effect
        cmds = []
        if any(p.startswith("/etc/sysctl.d/") for p in self.files):
            cmds.append(["sysctl", "--system"])
        if any(p.startswith("/etc/udev/rules.d/") for p in self.files):
            cmds.append(["udevadm", "control", "--reload"])
        if any(p.startswith("/etc/systemd/system.conf.d/") for p in self.files):
            cmds.append(["systemctl", "daemon-reexec"])
        if any(p.startswith("/etc/systemd/journald.conf.d/") for p in self.files):
            cmds.append(["systemctl", "restart", "systemd-journald"])
        return cmds


SYSCTL = ["sysctl", "--system"]

def _policy(policies: dict, wrap: bool = False) -> str:
    """JSON text for a browser managed-policy file. Firefox wraps the policies in a top-level "policies" key."""
    return json.dumps({"policies": policies} if wrap else policies, indent=2) + "\n"


# Policies understood by every Chromium-based browser (Chrome, Chromium, Brave, Edge). A browser ignores keys it does not know.
_CHROMIUM_PRIVACY = {
    "MetricsReportingEnabled": False, "SafeBrowsingExtendedReportingEnabled": False, "UrlKeyedAnonymizedDataCollectionEnabled": False,
    "SpellCheckServiceEnabled": False, "AlternateErrorPagesEnabled": False, "SearchSuggestEnabled": False, "BackgroundModeEnabled": False,
    "PromotionalTabsEnabled": False, "ShoppingListEnabled": False, "DefaultBrowserSettingEnabled": False, "NetworkPredictionOptions": 2,
    "PrivacySandboxPromptEnabled": False, "PrivacySandboxAdTopicsEnabled": False, "PrivacySandboxSiteEnabledAdsEnabled": False,
    "PrivacySandboxAdMeasurementEnabled": False, "WebRtcIPHandlingPolicy": "default_public_interface_only",
    "GenAiDefaultSettings": 2, "HelpMeWriteSettings": 2, "TabCompareSettings": 2, "HistorySearchSettings": 2, "GeminiSettings": 1,
}
_BRAVE = dict(_CHROMIUM_PRIVACY, BraveRewardsDisabled=True, BraveWalletDisabled=True, BraveVPNDisabled=True, BraveAIChatEnabled=False,
              BraveNewsDisabled=True, BraveTalkDisabled=True, BraveP3AEnabled=False, BraveStatsPingEnabled=False, BraveWebDiscoveryEnabled=False)
_EDGE = dict(_CHROMIUM_PRIVACY, DiagnosticData=0, EdgeShoppingAssistantEnabled=False, HubsSidebarEnabled=False, EdgeFollowEnabled=False,
             ShowRecommendationsEnabled=False, PersonalizationReportingEnabled=False, StartupBoostEnabled=False, NewTabPageContentEnabled=False,
             SpotlightExperiencesAndRecommendationsEnabled=False, EdgeWorkspacesEnabled=False, ShowMicrosoftRewards=False, UserFeedbackAllowed=False,
             Microsoft365CopilotChatIconEnabled=False)
_FIREFOX = {
    "DisableTelemetry": True, "DisableFirefoxStudies": True, "DisablePocket": True, "DisableFeedbackCommands": True, "DontCheckDefaultBrowser": True,
    "OverrideFirstRunPage": "", "OverridePostUpdatePage": "",
    "FirefoxSuggest": {"WebSuggestions": False, "SponsoredSuggestions": False, "ImproveSuggest": False},
    "FirefoxHome": {"SponsoredTopSites": False, "SponsoredPocket": False, "Pocket": False, "Stories": False, "SponsoredStories": False, "Snippets": False},
    "UserMessaging": {"ExtensionRecommendations": False, "FeatureRecommendations": False, "UrlbarInterventions": False, "SkipOnboarding": True, "MoreFromMozilla": False},
    "EnableTrackingProtection": {"Value": True, "Cryptomining": True, "Fingerprinting": True, "EmailTracking": True},
    "Preferences": {k: {"Value": False, "Status": "default"} for k in (
        "browser.ml.chat.enabled", "browser.ml.chat.sidebar", "browser.ml.linkPreview.enabled", "browser.tabs.groups.smart.enabled",
        "datareporting.healthreport.uploadEnabled", "app.shield.optoutstudies.enabled", "browser.newtabpage.activity-stream.feeds.telemetry",
        "browser.newtabpage.activity-stream.telemetry", "browser.discovery.enabled", "browser.newtabpage.activity-stream.showWeather",
        "browser.newtabpage.activity-stream.system.showWeather", "browser.newtabpage.activity-stream.showSponsored",
        "browser.newtabpage.activity-stream.showSponsoredTopSites", "browser.newtabpage.activity-stream.feeds.section.topstories",
        "browser.newtabpage.activity-stream.feeds.system.topstories", "browser.newtabpage.activity-stream.widgets.system.enabled",
        "browser.newtabpage.activity-stream.widgets.enabled", "browser.newtabpage.activity-stream.feeds.weatherfeed",
        "browser.newtabpage.activity-stream.discoverystream.sponsoredCollections.enabled")},
}
# Firefox's "Preferences" policy only accepts a short allow-list of settings and silently ignores the rest (the new-tab widgets, weather and
# sponsored stories are not on it). An autoconfig file has no such limit, so the new-tab and telemetry settings are locked this way.
_FF_NEWTAB = ("browser.newtabpage.activity-stream." + n for n in (
    "showSponsored", "showSponsoredTopSites", "system.showSponsored", "showWeather", "system.showWeather", "feeds.weatherfeed",
    "feeds.section.topstories", "feeds.system.topstories", "widgets.enabled", "widgets.system.enabled", "widgets.lists.enabled",
    "widgets.focusTimer.enabled", "discoverystream.enabled", "discoverystream.sponsoredCollections.enabled", "feeds.telemetry", "telemetry"))
_FF_LOCKED = tuple(_FF_NEWTAB) + (
    "browser.ml.chat.enabled", "browser.ml.chat.sidebar", "browser.ml.linkPreview.enabled", "browser.tabs.groups.smart.enabled",
    "extensions.pocket.enabled", "app.normandy.enabled", "app.shield.optoutstudies.enabled", "toolkit.telemetry.enabled",
    "toolkit.telemetry.unified", "toolkit.telemetry.archive.enabled", "datareporting.healthreport.uploadEnabled",
    "datareporting.policy.dataSubmissionEnabled", "browser.discovery.enabled", "browser.urlbar.suggest.quicksuggest.sponsored",
    "browser.urlbar.suggest.quicksuggest.nonsponsored")
_FF_AUTOCONFIG = 'pref("general.config.filename", "optimaxer.cfg");\npref("general.config.obscure_value", 0);\n'
_FF_CFG = "// Optimaxer: locked Firefox preferences. Undo the tweak in Optimaxer to remove this file.\n" + "".join(
    f'lockPref("{k}", false);\n' for k in _FF_LOCKED)
_POLICY_NOTE = " The browser shows 'managed by your organization'; any policy file you already have is saved and restored on undo."

TWEAKS = [
    # ---- performance
    Tweak("perf-vm", "Memory tuning (swappiness 20, cache pressure 50)", "Keeps apps in RAM longer and flushes dirty pages sooner: snappier desktop, smoother large copies.",
          "Performance", tags={"safe", "perf", "max"},
          files={"/etc/sysctl.d/99-optimaxer-vm.conf": "vm.swappiness = 20\nvm.vfs_cache_pressure = 50\nvm.dirty_ratio = 10\nvm.dirty_background_ratio = 5\n"}, post=[SYSCTL]),
    Tweak("perf-zram", "Compressed swap in RAM (zram)", "Creates a zstd-compressed swap device sized min(RAM/2, 8 GB): fewer freezes under memory pressure and less SSD wear. Installs zram-generator.",
          "Performance", tags={"safe", "perf", "max"}, pkgs=["zram-generator"],
          files={"/etc/systemd/zram-generator.conf": "[zram0]\nzram-size = min(ram / 2, 8192)\ncompression-algorithm = zstd\nswap-priority = 100\n",
                 "/etc/sysctl.d/99-optimaxer-zram.conf": "vm.swappiness = 100\nvm.page-cluster = 0\n"},
          post=[["systemctl", "daemon-reload"], ["systemctl", "start", "systemd-zram-setup@zram0.service"], SYSCTL],
          undo_post=[["systemctl", "stop", "systemd-zram-setup@zram0.service"], ["systemctl", "daemon-reload"]]),
    Tweak("perf-trim", "Weekly SSD TRIM", "Enables fstrim.timer so free blocks are trimmed once a week: keeps SSD speed up.", "Performance",
          tags={"safe", "perf", "max"}, enable=["fstrim.timer"], needs=["systemctl"]),
    Tweak("perf-iosched", "Best I/O scheduler per disk type", "NVMe: none, SATA SSD: mq-deadline, hard disk: bfq (udev rule).", "Performance", tags={"safe", "perf", "max"},
          files={"/etc/udev/rules.d/60-optimaxer-ioscheduler.rules":
                 'ACTION=="add|change", KERNEL=="nvme[0-9]*", ATTR{queue/scheduler}="none"\n'
                 'ACTION=="add|change", KERNEL=="sd[a-z]*|mmcblk[0-9]*", ATTR{queue/rotational}=="0", ATTR{queue/scheduler}="mq-deadline"\n'
                 'ACTION=="add|change", KERNEL=="sd[a-z]", ATTR{queue/rotational}=="1", ATTR{queue/scheduler}="bfq"\n'},
          post=[["udevadm", "control", "--reload"], ["udevadm", "trigger", "--subsystem-match=block", "--action=change"]]),
    Tweak("perf-journal", "Limit the system journal to 200 MB", "Stops /var/log/journal growing to gigabytes.", "Performance", tags={"safe", "perf", "max"},
          files={"/etc/systemd/journald.conf.d/99-optimaxer.conf": "[Journal]\nSystemMaxUse=200M\nRuntimeMaxUse=50M\n"},
          post=[["systemctl", "restart", "systemd-journald"]]),
    Tweak("perf-stoptimeout", "Faster shutdown (10 s service stop timeout)", "Hung services are killed after 10 s instead of 90 s.", "Performance", tags={"safe", "perf", "max"},
          files={"/etc/systemd/system.conf.d/99-optimaxer.conf": "[Manager]\nDefaultTimeoutStopSec=10s\nDefaultTimeoutStartSec=30s\n"},
          post=[["systemctl", "daemon-reexec"]]),
    Tweak("perf-waitonline", "Do not wait for the network at boot", "Disables NetworkManager-wait-online.service: boots a few seconds faster. Breaks network mounts that must be ready at boot.",
          "Performance", risk="moderate", tags={"max"}, disable=["NetworkManager-wait-online.service"], needs=["/usr/lib/systemd/system/NetworkManager-wait-online.service"]),
    Tweak("perf-earlyoom", "Early out-of-memory killer", "earlyoom kills the biggest app before the system freezes completely when RAM and swap run out.", "Performance",
          tags={"safe", "perf", "max"}, pkgs=["earlyoom"], enable=["earlyoom.service"]),
    Tweak("perf-coredump", "Disable core dumps", "Crashing apps stop writing large dump files (privacy and disk space).", "Performance", tags={"safe", "perf", "privacy", "max"},
          files={"/etc/systemd/coredump.conf.d/99-optimaxer.conf": "[Coredump]\nStorage=none\nProcessSizeMax=0\n"}),
    Tweak("perf-wifi-powersave", "Wi-Fi power saving off", "NetworkManager keeps the Wi-Fi radio awake: lower latency and fewer dropouts, slightly more battery use on laptops.", "Network",
          tags={"perf", "max"}, needs=["nmcli"],
          files={"/etc/NetworkManager/conf.d/99-optimaxer-wifi.conf": "[connection]\nwifi.powersave = 2\n"},
          post=[["systemctl", "reload", "NetworkManager"]], undo_post=[["systemctl", "reload", "NetworkManager"]]),
    Tweak("perf-nmi", "Disable the NMI watchdog", "Frees a little CPU time and saves power; only used for kernel debugging.", "Performance", tags={"safe", "perf", "max"},
          files={"/etc/sysctl.d/99-optimaxer-nmi.conf": "kernel.nmi_watchdog = 0\n"}, post=[SYSCTL]),
    Tweak("perf-grubtimeout", "Shorter GRUB menu timeout (2 s)", "Boots 3 seconds sooner. The menu is still reachable by holding Shift/Esc.", "Performance", risk="moderate", tags={"max"},
          edits=[LineEdit("/etc/default/grub", "GRUB_TIMEOUT", "2", sep="=")], needs=["/etc/default/grub"],
          post=[["sh", "-c", "command -v update-grub >/dev/null && update-grub || grub-mkconfig -o /boot/grub/grub.cfg"]],
          undo_post=[["sh", "-c", "command -v update-grub >/dev/null && update-grub || grub-mkconfig -o /boot/grub/grub.cfg"]]),
    # ---- package manager
    Tweak("pm-pacman", "pacman: parallel downloads and colour", "ParallelDownloads = 5 and Color in /etc/pacman.conf: much faster updates, readable output.", "Package manager",
          tags={"safe", "perf", "max"}, needs=["/etc/pacman.conf"],
          edits=[LineEdit("/etc/pacman.conf", "ParallelDownloads", "5", section="options"), LineEdit("/etc/pacman.conf", "Color", None, section="options")]),
    Tweak("pm-paccache", "Weekly package cache cleaning", "paccache.timer keeps the last 3 versions of each package in /var/cache/pacman/pkg. Installs pacman-contrib.", "Package manager",
          tags={"safe", "perf", "max"}, pkgs=["pacman-contrib"], enable=["paccache.timer"]),
    # ---- network
    Tweak("net-bbr", "TCP BBR congestion control", "Better throughput and latency on lossy links (Wi-Fi, long distance).", "Network", tags={"safe", "perf", "max"},
          files={"/etc/modules-load.d/optimaxer-bbr.conf": "tcp_bbr\n", "/etc/sysctl.d/99-optimaxer-net.conf": "net.core.default_qdisc = fq\nnet.ipv4.tcp_congestion_control = bbr\n"},
          post=[["modprobe", "tcp_bbr"], SYSCTL]),
    # ---- security / privacy
    Tweak("sec-sysctl", "Kernel and network hardening", "Hides kernel pointers and dmesg from users, enables reverse-path filtering, ignores ICMP redirects and broadcast pings.", "Security",
          tags={"safe", "privacy", "max"},
          files={"/etc/sysctl.d/99-optimaxer-harden.conf": "kernel.dmesg_restrict = 1\nkernel.kptr_restrict = 1\nnet.ipv4.conf.all.rp_filter = 1\nnet.ipv4.conf.default.rp_filter = 1\n"
                 "net.ipv4.icmp_echo_ignore_broadcasts = 1\nnet.ipv4.conf.all.accept_redirects = 0\nnet.ipv4.conf.default.accept_redirects = 0\n"
                 "net.ipv6.conf.all.accept_redirects = 0\nnet.ipv4.conf.all.send_redirects = 0\n"}, post=[SYSCTL]),
    Tweak("sec-ufw", "Firewall (ufw): deny incoming, allow outgoing", "Installs and enables ufw. If SSH is running its port is opened first. Blocks incoming connections such as KDE Connect, Samba or game servers until you allow them.",
          "Security", risk="moderate", tags={"max"}, pkgs=["ufw"], enable=["ufw.service"],
          cmds=[["ufw", "default", "deny", "incoming"], ["ufw", "default", "allow", "outgoing"],
                ["sh", "-c", "systemctl is-active --quiet sshd && ufw allow ssh || true"]],
          post=[["ufw", "--force", "enable"]], undo_post=[["ufw", "--force", "disable"]], test_cmd=["sh", "-c", "ufw status | grep -q 'Status: active'"]),
    Tweak("priv-avahi", "Disable mDNS/Avahi network discovery", "Stops the machine announcing itself on the LAN. Network printer and Chromecast auto-discovery stops working.", "Privacy",
          risk="moderate", tags={"privacy", "max"}, disable=["avahi-daemon.service", "avahi-daemon.socket"], needs=["/usr/lib/systemd/system/avahi-daemon.service"]),
    # ---- browser debloat (managed policies: they apply to every profile and survive browser updates)
    Tweak("browser-firefox", "Firefox: telemetry, studies, Pocket, sponsored content, AI chat", "Turns off telemetry, Shield studies, Pocket, sponsored tiles and suggestions, first-run pages, the AI chatbot and link previews; enables strict tracking protection. Also locks the new-tab weather, widgets, stories and sponsored tiles off through an autoconfig file in /usr/lib/firefox. Close every Firefox window and reopen it; about:policies should list the policies." + _POLICY_NOTE,
          "Browsers", tags={"privacy", "max"}, files={"/usr/lib/firefox/distribution/policies.json": _policy(_FIREFOX, wrap=True),
                                                  "/etc/firefox/policies/policies.json": _policy(_FIREFOX, wrap=True),
                                                  "/usr/lib/firefox/defaults/pref/optimaxer-autoconfig.js": _FF_AUTOCONFIG,
                                                  "/usr/lib/firefox/optimaxer.cfg": _FF_CFG}, needs=["firefox"]),
    Tweak("browser-chromium", "Chromium: reporting, ads API, prediction, AI features", "Turns off usage reporting, Privacy Sandbox ad APIs, the spelling web service, search suggestions, network prediction and generative-AI features; WebRTC no longer leaks the local IP." + _POLICY_NOTE,
          "Browsers", tags={"privacy", "max"}, files={"/etc/chromium/policies/managed/optimaxer.json": _policy(_CHROMIUM_PRIVACY)}, needs=["chromium"]),
    Tweak("browser-chrome", "Google Chrome: reporting, ads API, prediction, AI features", "Turns off usage reporting, Privacy Sandbox ad APIs, the spelling web service, search suggestions, network prediction, background mode and Gemini/generative-AI features." + _POLICY_NOTE,
          "Browsers", tags={"privacy", "max"}, files={"/etc/opt/chrome/policies/managed/optimaxer.json": _policy(_CHROMIUM_PRIVACY)}, needs=["google-chrome-stable"]),
    Tweak("browser-brave", "Brave: Rewards, Wallet, VPN, AI chat, News, Talk, telemetry", "Turns off Brave Rewards, Wallet, VPN, Leo AI chat, News, Talk, P3A and the stats ping, plus the shared Chromium privacy policies." + _POLICY_NOTE,
          "Browsers", tags={"privacy", "max"}, files={"/etc/brave/policies/managed/optimaxer.json": _policy(_BRAVE)}, needs=["brave"]),
    Tweak("browser-edge", "Microsoft Edge: diagnostics, shopping, sidebar, Copilot, rewards", "Turns off diagnostic data, the shopping assistant, sidebar, Copilot icon, recommendations, startup boost, workspaces and Rewards, plus the shared Chromium privacy policies." + _POLICY_NOTE,
          "Browsers", tags={"privacy", "max"}, files={"/etc/opt/edge/policies/managed/optimaxer.json": _policy(_EDGE)}, needs=["microsoft-edge-stable"]),
]

PRESETS = ("safe", "perf", "privacy", "max")


def tweak_by_id(tid: str):
    return next((t for t in TWEAKS if t.id == tid), None)


def preset_ids(preset: str) -> list:
    return [t.id for t in TWEAKS if preset in t.tags and t.available()]


# ======================================================================================================== apps (pacman / AUR / flatpak)

APPS_RAW = """
Browsers|Firefox|P|firefox
Browsers|Chromium|P|chromium
Browsers|Vivaldi|P|vivaldi
Browsers|Falkon|P|falkon
Browsers|Tor Browser|P|torbrowser-launcher
Browsers|Brave|A|brave-bin
Browsers|Google Chrome|A|google-chrome
Browsers|Zen Browser|A|zen-browser-bin
Browsers|Floorp|A|floorp-bin
Browsers|Opera|A|opera
Communications|Discord|P|discord
Communications|Telegram|P|telegram-desktop
Communications|Signal|P|signal-desktop
Communications|Thunderbird|P|thunderbird
Communications|Element|P|element-desktop
Communications|Slack|A|slack-desktop
Communications|Zoom|A|zoom
Communications|Teams for Linux|A|teams-for-linux
Communications|Vesktop|A|vesktop-bin
Development|Git|P|git
Development|GitHub CLI|P|github-cli
Development|Base development tools|P|base-devel
Development|Neovim|P|neovim
Development|Helix|P|helix
Development|Zed|P|zed
Development|Python|P|python
Development|Node.js|P|nodejs
Development|npm|P|npm
Development|Go|P|go
Development|Rust (rustup)|P|rustup
Development|Docker|P|docker
Development|Docker Compose|P|docker-compose
Development|DBeaver|P|dbeaver
Development|Lazygit|P|lazygit
Development|VS Code|A|visual-studio-code-bin
Development|VSCodium|A|vscodium-bin
Development|Cursor|A|cursor-bin
Development|Postman|A|postman-bin
Development|JetBrains Toolbox|A|jetbrains-toolbox
Documents|LibreOffice|P|libreoffice-fresh
Documents|Okular|P|okular
Documents|Evince|P|evince
Documents|Obsidian|P|obsidian
Documents|Foliate|P|foliate
Documents|ONLYOFFICE|A|onlyoffice-bin
Documents|Joplin|A|joplin-desktop
Documents|Zotero|A|zotero-bin
Games|Steam|P|steam
Games|Lutris|P|lutris
Games|Wine|P|wine
Games|GameMode|P|gamemode
Games|MangoHud|P|mangohud
Games|RetroArch|P|retroarch
Games|Prism Launcher|P|prismlauncher
Games|Heroic Games Launcher|A|heroic-games-launcher-bin
Games|ProtonUp-Qt|A|protonup-qt
Games|Bottles|F|com.usebottles.bottles
Multimedia|VLC|P|vlc
Multimedia|mpv|P|mpv
Multimedia|OBS Studio|P|obs-studio
Multimedia|GIMP|P|gimp
Multimedia|Inkscape|P|inkscape
Multimedia|Krita|P|krita
Multimedia|Kdenlive|P|kdenlive
Multimedia|Blender|P|blender
Multimedia|Audacity|P|audacity
Multimedia|HandBrake|P|handbrake
Multimedia|Strawberry|P|strawberry
Multimedia|Flameshot|P|flameshot
Multimedia|FFmpeg|P|ffmpeg
Multimedia|Spotify|A|spotify
Utilities|htop|P|htop
Utilities|btop|P|btop
Utilities|fastfetch|P|fastfetch
Utilities|Timeshift|P|timeshift
Utilities|BleachBit|P|bleachbit
Utilities|Bitwarden|P|bitwarden
Utilities|KeePassXC|P|keepassxc
Utilities|qBittorrent|P|qbittorrent
Utilities|Syncthing|P|syncthing
Utilities|Tailscale|P|tailscale
Utilities|WireGuard tools|P|wireguard-tools
Utilities|Wireshark|P|wireshark-qt
Utilities|Nmap|P|nmap
Utilities|GParted|P|gparted
Utilities|7-Zip|P|7zip
Utilities|unrar|P|unrar
Utilities|CopyQ|P|copyq
Utilities|Flatpak|P|flatpak
Utilities|yay (AUR helper)|P|yay
Utilities|TLP (laptop power)|P|tlp
Utilities|power-profiles-daemon|P|power-profiles-daemon
Utilities|AnyDesk|A|anydesk-bin
Utilities|balenaEtcher|A|balena-etcher
Utilities|Ventoy|A|ventoy-bin
Utilities|Flatseal|F|com.github.tchx84.Flatseal
"""


def load_apps() -> list:
    apps = []
    for line in APPS_RAW.strip().splitlines():
        cat, name, src, pkg = line.split("|")
        apps.append({"cat": cat, "name": name, "src": src, "pkg": pkg})
    return apps


APPS = load_apps()


def aur_helper() -> str | None:
    for h in ("yay", "paru"):
        if have(h):
            return h
    return None


def app_installed(app: dict) -> bool:
    if app["src"] == "F":
        return flatpak_installed(app["pkg"])
    return pkg_installed(app["pkg"])


def install_app(app: dict) -> bool:
    name, pkg, src = app["name"], app["pkg"], app["src"]
    log(f"Install {name} ({src}:{pkg})")
    if app_installed(app):
        log("  already installed")
        return True
    if src == "P":
        rc, out = run(["pacman", "-S", "--needed", "--noconfirm", pkg], write=True)
    elif src == "A":
        helper = aur_helper()
        if not helper:
            log("  needs an AUR helper: install 'yay' from the Install tab first", "WARN")
            return False
        rc, out = run([helper, "-S", "--needed", "--noconfirm", pkg], write=True, as_user=True)
    else:
        if not have("flatpak"):
            log("  needs flatpak: install 'Flatpak' from the Utilities group first", "WARN")
            return False
        rc, out = run(["flatpak", "install", "-y", "--noninteractive", "flathub", pkg], write=True)
    if CTX.dry_run:
        return True
    ok = rc == 0 and app_installed(app)
    if ok:
        log("  installed and verified", "OK")
    else:
        tail = out.strip().splitlines()[-1] if out.strip() else f"exit {rc}"
        log(f"  FAILED: {tail}", "WARN")
    return ok


def remove_app(app: dict) -> bool:
    name, pkg, src = app["name"], app["pkg"], app["src"]
    log(f"Remove {name}")
    if not app_installed(app):
        log("  not installed")
        return True
    if src == "F":
        rc, out = run(["flatpak", "uninstall", "-y", "--noninteractive", pkg], write=True)
    else:
        rc, out = run(["pacman", "-Rns", "--noconfirm", pkg], write=True)
    if CTX.dry_run:
        return True
    ok = rc == 0 and not app_installed(app)
    log("  removed and verified" if ok else f"  FAILED: {out.strip()[-160:]}", "OK" if ok else "WARN")
    return ok


# ======================================================================================================== debloat

DEBLOAT = [
    ("manjaro-hello", "Manjaro Hello (welcome screen)"), ("hplip", "HP printer drivers (only needed for HP printers)"),
    ("simple-scan", "Simple Scan"), ("sane", "Scanner support (SANE)"), ("thunderbird", "Thunderbird mail client"),
    ("libreoffice-fresh", "LibreOffice (fresh)"), ("libreoffice-still", "LibreOffice (still)"), ("gimp", "GIMP"),
    ("inkscape", "Inkscape"), ("kdenlive", "Kdenlive video editor"), ("vlc", "VLC"), ("epiphany", "GNOME Web"),
    ("totem", "GNOME Videos"), ("gnome-weather", "GNOME Weather"), ("gnome-maps", "GNOME Maps"),
    ("cheese", "Cheese webcam app"), ("pidgin", "Pidgin"), ("konversation", "Konversation IRC"), ("kmail", "KMail"),
]


def installed_debloat() -> list:
    out = []
    for pkg, label in DEBLOAT:
        if pkg_installed(pkg):
            rc, info = run(["pacman", "-Qi", pkg])
            size = parse_installed_size(info)
            out.append({"pkg": pkg, "label": label, "size": size})
    return out


def parse_installed_size(info: str) -> int:
    m = re.search(r"Installed Size\s*:\s*([\d.,]+)\s*(B|KiB|MiB|GiB)", info or "")
    if not m:
        return 0
    n = float(m.group(1).replace(",", "."))
    return int(n * {"B": 1, "KiB": 1024, "MiB": 1024 ** 2, "GiB": 1024 ** 3}[m.group(2)])


# ======================================================================================================== cleaner

def _home(p: str) -> str:
    return os.path.join(real_home(), p)


def orphans() -> list:
    rc, out = run(["pacman", "-Qtdq"])
    return [l.strip() for l in out.splitlines() if l.strip()] if rc == 0 else []


CLEAN_TARGETS = [
    {"id": "pacman-cache", "name": "pacman package cache (keep 2 newest versions)", "paths": ["/var/cache/pacman/pkg"], "risk": "safe"},
    {"id": "orphans", "name": "Orphaned packages (installed as dependencies, no longer needed)", "special": "orphans", "risk": "moderate"},
    {"id": "journal", "name": "systemd journal (keep 100 MB)", "paths": ["/var/log/journal"], "risk": "safe"},
    {"id": "coredumps", "name": "Core dumps", "paths": ["/var/lib/systemd/coredump"], "risk": "safe"},
    {"id": "yay", "name": "AUR helper build cache (yay/paru)", "home": [".cache/yay", ".cache/paru"], "risk": "safe"},
    {"id": "thumbs", "name": "Thumbnail cache", "home": [".cache/thumbnails"], "risk": "safe"},
    {"id": "trash", "name": "Trash", "home": [".local/share/Trash/files", ".local/share/Trash/info"], "risk": "moderate"},
    {"id": "firefox", "name": "Firefox cache", "home": [".cache/mozilla/firefox"], "risk": "safe"},
    {"id": "chromium", "name": "Chromium / Chrome / Brave / Vivaldi cache", "home": [".cache/chromium", ".cache/google-chrome", ".cache/BraveSoftware", ".cache/vivaldi"], "risk": "safe"},
    {"id": "pip", "name": "pip cache", "home": [".cache/pip"], "risk": "safe"},
    {"id": "npm", "name": "npm cache", "home": [".npm/_cacache"], "risk": "safe"},
    {"id": "go", "name": "Go build cache", "home": [".cache/go-build"], "risk": "safe"},
    {"id": "cargo", "name": "Rust cargo download cache", "home": [".cargo/registry/cache"], "risk": "moderate"},
    {"id": "flatpak-unused", "name": "Unused Flatpak runtimes", "special": "flatpak", "risk": "safe"},
]


def target_paths(t: dict) -> list:
    return list(t.get("paths", [])) + [_home(h) for h in t.get("home", [])]


def target_size(t: dict) -> int:
    sp = t.get("special")
    if sp == "orphans":
        total = 0
        for pkg in orphans():
            total += parse_installed_size(run(["pacman", "-Qi", pkg])[1])
        return total
    if sp == "flatpak":
        return -1
    return sum(dir_size(p) for p in target_paths(t))


def clean_target(t: dict) -> int:
    """Clean one target; returns bytes freed (best effort)."""
    before = max(target_size(t), 0)
    tid = t["id"]
    log(f"Clean: {t['name']}")
    if tid == "pacman-cache":
        if have("paccache"):
            run(["paccache", "-rk2"], write=True)
            run(["paccache", "-ruk0"], write=True)
        else:
            run(["pacman", "-Sc", "--noconfirm"], write=True)
    elif tid == "orphans":
        pk = orphans()
        if pk:
            run(["pacman", "-Rns", "--noconfirm"] + pk, write=True)
    elif tid == "journal":
        run(["journalctl", "--vacuum-size=100M"], write=True)
    elif tid == "flatpak-unused":
        if have("flatpak"):
            run(["flatpak", "uninstall", "--unused", "-y"], write=True)
    else:
        for p in target_paths(t):
            full = P(p)
            if not os.path.isdir(full):
                continue
            for entry in os.listdir(full):
                path = os.path.join(full, entry)
                if CTX.dry_run:
                    log(f"[dry-run] remove {path}", "DRY")
                    continue
                try:
                    shutil.rmtree(path) if os.path.isdir(path) and not os.path.islink(path) else os.remove(path)
                except OSError:
                    pass
    after = 0 if CTX.dry_run else max(target_size(t), 0)
    return max(before - after, 0)


# ======================================================================================================== services

SERVICES = [
    ("NetworkManager-wait-online.service", "Boot waits until the network is up", "disable if you have no network mounts"),
    ("avahi-daemon.service", "mDNS/Zeroconf discovery", "disable unless you use network printers or Chromecast"),
    ("avahi-daemon.socket", "Avahi socket activation", "disable together with avahi-daemon"),
    ("ModemManager.service", "Mobile broadband modem support", "disable on desktops without a modem"),
    ("bluetooth.service", "Bluetooth", "disable if you never use Bluetooth"),
    ("cups.service", "Printing (CUPS)", "disable if you never print"),
    ("cups.socket", "CUPS socket activation", "disable together with cups"),
    ("cups-browsed.service", "Network printer discovery", "disable unless you print to network printers"),
    ("smb.service", "Samba file sharing server", "disable unless you share files with Windows"),
    ("sshd.service", "SSH server", "disable unless you log in remotely"),
    ("docker.service", "Docker daemon", "disable and start on demand if you rarely use containers"),
    ("tlp.service", "TLP laptop power management", "leave enabled on laptops"),
    ("fstrim.timer", "Weekly SSD TRIM", "leave enabled on SSDs"),
    ("paccache.timer", "Weekly package cache cleaning", "leave enabled"),
    ("lvm2-monitor.service", "LVM monitoring", "disable if you do not use LVM"),
]


def service_rows() -> list:
    rows = []
    for unit, label, hint in SERVICES:
        if not unit_exists(unit):
            continue
        rows.append({"unit": unit, "label": label, "hint": hint, "enabled": unit_enabled(unit), "active": unit_active(unit)})
    return rows


def set_service(unit: str, enable: bool) -> bool:
    state = load_state()
    key = f"service:{unit}"
    if key not in state:
        state[key] = {"unit": unit, "enabled": unit_enabled(unit), "active": unit_active(unit)}
        save_state(state)
    run(["systemctl", "enable" if enable else "disable", "--now", unit], write=True)
    ok = CTX.dry_run or (unit_enabled(unit) == enable)
    log(f"{'Enabled' if enable else 'Disabled'} {unit}" + ("" if ok else " (state did not change)"), "OK" if ok else "WARN")
    return ok


def restore_service(unit: str) -> bool:
    state = load_state()
    snap = state.get(f"service:{unit}")
    if not snap:
        log(f"No saved state for {unit}", "WARN")
        return False
    run(["systemctl", "enable" if snap["enabled"] else "disable"] + (["--now"] if snap["active"] == snap["enabled"] else []) + [unit], write=True)
    state.pop(f"service:{unit}", None)
    save_state(state)
    log(f"Restored {unit}", "OK")
    return True


# ======================================================================================================== startup (XDG autostart)

MARK = "# optimaxer-override"


def parse_desktop(text: str) -> dict:
    d = {}
    for line in text.splitlines():
        if "=" in line and not line.strip().startswith("#"):
            k, v = line.split("=", 1)
            d.setdefault(k.strip(), v.strip())
    return d


def autostart_items() -> list:
    items, seen = [], set()
    user_dir = _home(".config/autostart")
    for scope, d in (("user", user_dir), ("system", "/etc/xdg/autostart")):
        full = P(d)
        if not os.path.isdir(full):
            continue
        for f in sorted(os.listdir(full)):
            if not f.endswith(".desktop") or f in seen:
                continue
            seen.add(f)
            text = read_text(os.path.join(d, f)) or ""
            if scope == "system":
                override = read_text(os.path.join(user_dir, f))
                if override is not None:
                    text = override
            kv = parse_desktop(text)
            hidden = kv.get("Hidden", "false").lower() == "true" or kv.get("X-GNOME-Autostart-enabled", "true").lower() == "false"
            items.append({"file": f, "name": kv.get("Name", f), "exec": kv.get("Exec", ""), "scope": scope, "enabled": not hidden})
    return items


def set_autostart(item: dict, enable: bool) -> bool:
    user_dir = _home(".config/autostart")
    path = os.path.join(user_dir, item["file"])
    sys_path = os.path.join("/etc/xdg/autostart", item["file"])
    current = read_text(path)
    if current is None:
        base = read_text(sys_path)
        if base is None:
            return False
        current = base.rstrip("\n") + "\n" + MARK + "\n"
    lines = [l for l in current.split("\n") if not re.match(r"^\s*Hidden\s*=", l)]
    if not enable:
        idx = next((i for i, l in enumerate(lines) if l.strip() == "[Desktop Entry]"), -1)
        lines.insert(idx + 1, "Hidden=true")
    write_text(path, "\n".join(lines))
    if not CTX.dry_run and hasattr(os, "geteuid") and os.geteuid() == 0 and not CTX.root:
        try:
            import pwd
            pw = pwd.getpwnam(real_user())
            os.chown(path, pw.pw_uid, pw.pw_gid)
        except Exception:
            pass
    log(f"{'Enabled' if enable else 'Disabled'} autostart: {item['name']}", "OK")
    return True


# ======================================================================================================== network / DNS

DNS = {
    "cloudflare": ("Cloudflare", ["1.1.1.1", "1.0.0.1"], ["2606:4700:4700::1111", "2606:4700:4700::1001"]),
    "google": ("Google", ["8.8.8.8", "8.8.4.4"], ["2001:4860:4860::8888", "2001:4860:4860::8844"]),
    "quad9": ("Quad9", ["9.9.9.9", "149.112.112.112"], ["2620:fe::fe", "2620:fe::9"]),
    "adguard": ("AdGuard", ["94.140.14.14", "94.140.15.15"], ["2a10:50c0::ad1:ff", "2a10:50c0::ad2:ff"]),
    "opendns": ("OpenDNS", ["208.67.222.222", "208.67.220.220"], []),
    "auto": ("Automatic (DHCP)", [], []),
}


def active_connections() -> list:
    rc, out = run(["nmcli", "-t", "-f", "NAME,TYPE,DEVICE", "connection", "show", "--active"])
    conns = []
    if rc != 0:
        return conns
    for line in out.splitlines():
        parts = line.split(":")
        if len(parts) >= 3 and parts[1] in ("802-3-ethernet", "802-11-wireless", "ethernet", "wifi"):
            conns.append({"name": parts[0], "type": parts[1], "device": parts[2]})
    return conns


def set_dns(key: str) -> bool:
    if key not in DNS:
        log(f"Unknown DNS provider {key}", "ERROR")
        return False
    if not have("nmcli"):
        log("NetworkManager (nmcli) not found: set DNS in your network manager instead", "WARN")
        return False
    label, v4, v6 = DNS[key]
    state = load_state()
    ok_all = True
    for c in active_connections():
        sk = f"dns:{c['name']}"
        if sk not in state:
            def get(prop):
                return run(["nmcli", "-g", prop, "connection", "show", c["name"]])[1].strip()
            state[sk] = {"name": c["name"], "ipv4.dns": get("ipv4.dns"), "ipv4.ignore-auto-dns": get("ipv4.ignore-auto-dns"),
                         "ipv6.dns": get("ipv6.dns"), "ipv6.ignore-auto-dns": get("ipv6.ignore-auto-dns")}
            save_state(state)
        args = ["nmcli", "connection", "modify", c["name"], "ipv4.dns", ",".join(v4), "ipv4.ignore-auto-dns", "yes" if v4 else "no",
                "ipv6.dns", ",".join(v6), "ipv6.ignore-auto-dns", "yes" if v6 else "no"]
        rc, out = run(args, write=True)
        if rc == 0:
            run(["nmcli", "device", "reapply", c["device"]], write=True)
            log(f"DNS on {c['name']}: {label}", "OK")
        else:
            ok_all = False
            log(f"DNS on {c['name']}: {out.strip()[-100:]}", "WARN")
    if have("resolvectl"):
        run(["resolvectl", "flush-caches"], write=True)
    return ok_all


def restore_dns() -> bool:
    state = load_state()
    names = [k for k in state if k.startswith("dns:")]
    if not names:
        log("No saved DNS settings", "WARN")
        return False
    for k in names:
        s = state.pop(k)
        run(["nmcli", "connection", "modify", s["name"], "ipv4.dns", s["ipv4.dns"].replace(" ", ","), "ipv4.ignore-auto-dns", s["ipv4.ignore-auto-dns"] or "no",
             "ipv6.dns", s["ipv6.dns"].replace(" ", ","), "ipv6.ignore-auto-dns", s["ipv6.ignore-auto-dns"] or "no"], write=True)
        log(f"Restored DNS on {s['name']}", "OK")
    save_state(state)
    return True


# ======================================================================================================== tools

def sysinfo() -> list:
    d = distro()
    mem = read_text("/proc/meminfo") or ""
    m = re.search(r"MemTotal:\s+(\d+) kB", mem)
    cpu = re.search(r"model name\s*:\s*(.+)", read_text("/proc/cpuinfo") or "")
    kernel = run(["uname", "-r"])[1].strip()
    rows = [("Distro", d.get("PRETTY_NAME", "unknown")), ("Kernel", kernel), ("CPU", cpu.group(1) if cpu else "unknown"),
            ("Memory", fmt_size(int(m.group(1)) * 1024) if m else "unknown")]
    rc, out = run(["df", "-h", "--output=target,size,used,avail", "/"])
    if rc == 0 and out.strip():
        rows.append(("Root disk", " ".join(out.strip().splitlines()[-1].split())))
    rows.append(("Optimaxer", __version__))
    return rows


def tool_update_system() -> bool:
    log("Updating the system (pacman -Syu)...")
    rc, out = run(["pacman", "-Syu", "--noconfirm"], write=True)
    log("System updated" if rc == 0 else f"Update failed: {out.strip()[-160:]}", "OK" if rc == 0 else "WARN")
    return rc == 0


def tool_rank_mirrors() -> bool:
    if not have("pacman-mirrors"):
        log("pacman-mirrors is Manjaro-specific and was not found; on Arch use reflector", "WARN")
        return False
    rc, out = run(["pacman-mirrors", "--fasttrack", "10"], write=True)
    if rc == 0:
        run(["pacman", "-Syy"], write=True)
    log("Mirrors ranked and databases refreshed" if rc == 0 else f"Mirror ranking failed: {out.strip()[-120:]}", "OK" if rc == 0 else "WARN")
    return rc == 0


def tool_failed_units() -> list:
    rc, out = run(["systemctl", "--failed", "--no-legend", "--plain"])
    return [l.split()[0] for l in out.splitlines() if l.strip()]


def tool_pacnew() -> list:
    rc, out = run(["find", "/etc", "-name", "*.pacnew", "-o", "-name", "*.pacsave"])
    return [l for l in out.splitlines() if l.strip()]


def tool_smart() -> list:
    if not have("smartctl"):
        return ["smartctl not installed (pacman -S smartmontools)"]
    rows = []
    rc, out = run(["lsblk", "-dno", "NAME,TYPE"])
    for line in out.splitlines():
        parts = line.split()
        if len(parts) == 2 and parts[1] == "disk" and not parts[0].startswith(("loop", "zram")):
            h = run(["smartctl", "-H", f"/dev/{parts[0]}"])[1]
            m = re.search(r"overall-health[^:]*:\s*(\S+)", h)
            rows.append(f"/dev/{parts[0]}: {m.group(1) if m else 'unknown'}")
    return rows


# ======================================================================================================== self update

def _http_json(url: str):
    req = urllib.request.Request(url, headers={"User-Agent": "optimaxer-linux", "Accept": "application/vnd.github+json"})
    with urllib.request.urlopen(req, timeout=15) as r:
        return json.load(r)


def vtuple(v: str) -> tuple:
    return tuple(int(x) for x in re.findall(r"\d+", v)[:3])


def latest_release() -> dict:
    try:
        rel = _http_json(f"https://api.github.com/repos/{REPO}/releases/latest")
    except Exception as e:
        return {"error": str(e)}
    asset = next((a for a in rel.get("assets", []) if a["name"] == "optimaxer-linux.py"), None)
    if not asset:
        return {"error": "latest release has no optimaxer-linux.py asset"}
    return {"tag": rel["tag_name"], "newer": vtuple(rel["tag_name"]) > vtuple(__version__), "url": asset["browser_download_url"], "digest": asset.get("digest", "")}


def self_update(target: str | None = None) -> bool:
    rel = latest_release()
    if "error" in rel:
        log(f"Update check failed: {rel['error']}", "WARN")
        return False
    if not rel["newer"]:
        log(f"Optimaxer is up to date (v{__version__}).", "OK")
        return True
    log(f"Updating v{__version__} -> {rel['tag']}")
    req = urllib.request.Request(rel["url"], headers={"User-Agent": "optimaxer-linux"})
    data = urllib.request.urlopen(req, timeout=60).read()
    m = re.match(r"sha256:([0-9a-f]{64})", rel["digest"] or "")
    if m and hashlib.sha256(data).hexdigest() != m.group(1):
        log("Update rejected: checksum mismatch", "ERROR")
        return False
    if b"__version__" not in data or b"def main" not in data:
        log("Update rejected: not an Optimaxer script", "ERROR")
        return False
    dest = target or os.path.realpath(sys.argv[0])
    tmp = dest + ".new"
    with open(tmp, "wb") as fh:
        fh.write(data)
    os.chmod(tmp, 0o755)
    os.replace(tmp, dest)
    log(f"Updated {dest} to {rel['tag']}. Run it again.", "OK")
    return True


# ======================================================================================================== interactive UI

def ask(prompt: str) -> str:
    try:
        return input(prompt).strip()
    except EOFError:
        return "b"


def confirm(question: str) -> bool:
    if CTX.assume_yes:
        return True
    return ask(f"{question} [y/N] ").lower() in ("y", "yes")


def _text(rendered) -> str:
    return rendered[0] if isinstance(rendered, tuple) else rendered


def pick(title: str, rows: list, render, actions: str, presets: list | None = None, initial: set | None = None) -> tuple:
    """Generic checklist. Returns (command, selected indexes). rows are shown as numbered lines."""
    selected, flt = set(initial or ()), ""
    while True:
        print(f"\n== {title} ==")
        shown = [i for i, r in enumerate(rows) if not flt or flt.lower() in _text(render(r)).lower()]
        last_group = None
        for i in shown:
            line, group = render(rows[i]), None
            if isinstance(line, tuple):
                line, group = line
            if group and group != last_group:
                print(f"\n  # {group}")
                last_group = group
            print(f" {'[x]' if i in selected else '[ ]'} {i + 1:>3}. {line}")
        print(f"\n  numbers/ranges toggle (e.g. 1 4 7-9) | a all | n none | /text filter | {actions} | b back")
        if presets:
            print("  presets: " + ", ".join(f"p {p}" for p in presets))
        cmd = ask("> ")
        if cmd in ("b", "back", "q"):
            return "b", set()
        if cmd == "a":
            selected = set(shown)
        elif cmd == "n":
            selected = set()
        elif cmd.startswith("/"):
            flt = cmd[1:]
        elif cmd == "":
            flt = ""
        elif presets and cmd.startswith("p "):
            return cmd, selected
        elif cmd in actions.split(" | ") or cmd.split()[0] in [a.split()[0] for a in actions.split(" | ")]:
            return cmd.split()[0], selected
        else:
            for tok in cmd.replace(",", " ").split():
                m = re.match(r"^(\d+)-(\d+)$", tok)
                if m:
                    for n in range(int(m.group(1)), int(m.group(2)) + 1):
                        if 1 <= n <= len(rows):
                            selected ^= {n - 1}
                elif tok.isdigit() and 1 <= int(tok) <= len(rows):
                    selected ^= {int(tok) - 1}


def menu_tweaks() -> None:
    carry: set = set()
    while True:
        rows = [t for t in TWEAKS if t.available()]
        applied = {t.id: t.is_applied() for t in rows}
        render = lambda t: (f"{t.name}  [{t.risk}]{'  APPLIED' if applied[t.id] else ''}\n         {t.desc}", t.cat)
        cmd, sel = pick("Tweaks", rows, render, "apply | undo", presets=list(PRESETS), initial=carry)
        if cmd == "b":
            return
        if cmd.startswith("p "):
            ids = set(preset_ids(cmd.split()[1]))
            carry = {i for i, t in enumerate(rows) if t.id in ids}
            print(f"  preset selects {len(carry)} tweaks; use apply to run them")
            continue
        carry = set()
        chosen = [rows[i] for i in sorted(sel)]
        if not chosen:
            print("  nothing selected")
            continue
        if cmd == "apply" and confirm(f"Apply {len(chosen)} tweak(s)?"):
            for t in chosen:
                t.apply()
        if cmd == "undo" and confirm(f"Undo {len(chosen)} tweak(s)?"):
            for t in chosen:
                t.undo()


def menu_apps() -> None:
    while True:
        installed = {a["pkg"]: app_installed(a) for a in APPS}
        render = lambda a: (f"{a['name']}  ({a['src']}:{a['pkg']}){'  INSTALLED' if installed[a['pkg']] else ''}", a["cat"])
        cmd, sel = pick("Install apps (P = official repo, A = AUR, F = Flatpak)", APPS, render, "install | remove | upgrade")
        if cmd == "b":
            return
        if cmd == "upgrade":
            tool_update_system()
            continue
        chosen = [APPS[i] for i in sorted(sel)]
        if not chosen:
            print("  nothing selected")
            continue
        if cmd == "install" and confirm(f"Install {len(chosen)} app(s)?"):
            res = [install_app(a) for a in chosen]
            log(f"Install finished: {sum(res)} ok, {len(res) - sum(res)} failed", "OK" if all(res) else "WARN")
        if cmd == "remove" and confirm(f"Remove {len(chosen)} app(s)?"):
            res = [remove_app(a) for a in chosen]
            log(f"Remove finished: {sum(res)} ok, {len(res) - sum(res)} failed", "OK" if all(res) else "WARN")


def menu_cleaner() -> None:
    print("Scanning...")
    sizes = {t["id"]: target_size(t) for t in CLEAN_TARGETS}
    rows = CLEAN_TARGETS
    render = lambda t: f"{t['name']}  [{t['risk']}]  {'on demand' if sizes[t['id']] < 0 else fmt_size(sizes[t['id']])}"
    cmd, sel = pick("Cleaner", rows, render, "clean")
    if cmd != "clean" or not sel:
        return
    if confirm(f"Clean {len(sel)} item(s)? This cannot be undone."):
        freed = sum(clean_target(rows[i]) for i in sorted(sel))
        log(f"Total freed: {fmt_size(freed)}", "OK")


def menu_services() -> None:
    while True:
        rows = service_rows()
        render = lambda r: f"{r['unit']}  {'enabled' if r['enabled'] else 'disabled'}{' (running)' if r['active'] else ''}\n         {r['label']}: {r['hint']}"
        cmd, sel = pick("Services", rows, render, "enable | disable | restore")
        if cmd == "b":
            return
        for i in sorted(sel):
            if cmd == "enable":
                set_service(rows[i]["unit"], True)
            elif cmd == "disable":
                set_service(rows[i]["unit"], False)
            elif cmd == "restore":
                restore_service(rows[i]["unit"])


def menu_startup() -> None:
    while True:
        rows = autostart_items()
        render = lambda r: (f"{r['name']}  {'enabled' if r['enabled'] else 'disabled'}\n         {r['exec']}", r["scope"])
        cmd, sel = pick("Startup (desktop autostart entries)", rows, render, "disable | enable")
        if cmd == "b":
            return
        for i in sorted(sel):
            set_autostart(rows[i], cmd == "enable")


def menu_debloat() -> None:
    rows = installed_debloat()
    if not rows:
        print("None of the known optional packages are installed.")
        return
    render = lambda r: f"{r['label']} ({r['pkg']})  {fmt_size(r['size'])}"
    cmd, sel = pick("Debloat: optional packages you may not need (nothing is pre-selected)", rows, render, "remove")
    if cmd == "remove" and sel and confirm(f"Remove {len(sel)} package(s) with pacman -Rns?"):
        for i in sorted(sel):
            ok = run(["pacman", "-Rns", "--noconfirm", rows[i]["pkg"]], write=True)[0] == 0
            log(f"{rows[i]['pkg']}: {'removed' if ok else 'not removed (other packages depend on it?)'}", "OK" if ok else "WARN")


def menu_network() -> None:
    keys = list(DNS)
    render = lambda k: f"{DNS[k][0]}  {', '.join(DNS[k][1]) or 'router default'}"
    cmd, sel = pick("DNS provider (applies to active NetworkManager connections)", keys, render, "set | restore")
    if cmd == "restore":
        restore_dns()
    elif cmd == "set" and len(sel) == 1:
        set_dns(keys[next(iter(sel))])
    elif cmd == "set":
        print("  select exactly one provider")


def menu_tools() -> None:
    tools = [("System information", None), ("Update the system (pacman -Syu)", tool_update_system), ("Rank mirrors (Manjaro fasttrack)", tool_rank_mirrors),
             ("Show failed systemd units", None), ("Find .pacnew / .pacsave files", None), ("Disk health (SMART)", None), ("Undo everything Optimaxer changed", None),
             ("Check for an Optimaxer update", self_update)]
    while True:
        print("\n== Tools ==")
        for i, (n, _f) in enumerate(tools, 1):
            print(f"  {i}. {n}")
        c = ask("number (b = back) > ")
        if c in ("b", "back", "q", ""):
            return
        if not c.isdigit() or not 1 <= int(c) <= len(tools):
            continue
        n = int(c)
        if n == 1:
            for k, v in sysinfo():
                print(f"  {k:<10} {v}")
        elif n == 4:
            print("  " + (", ".join(tool_failed_units()) or "no failed units"))
        elif n == 5:
            print("\n".join("  " + l for l in tool_pacnew()) or "  none")
        elif n == 6:
            print("\n".join("  " + l for l in tool_smart()))
        elif n == 7:
            undo_all()
        else:
            tools[n - 1][1]()


def undo_all() -> None:
    state = load_state()
    ids = [k for k in state if tweak_by_id(k)]
    if not confirm(f"Undo {len(ids)} tweak(s) and restore saved services/DNS?"):
        return
    for k in ids:
        tweak_by_id(k).undo()
    for k in [k for k in load_state() if k.startswith("service:")]:
        restore_service(k.split(":", 1)[1])
    if any(k.startswith("dns:") for k in load_state()):
        restore_dns()


def interactive() -> None:
    d = distro()
    print(f"\nOptimaxer {__version__}  -  {d.get('PRETTY_NAME', 'Linux')}")
    if CTX.dry_run:
        print("DRY RUN: nothing will be changed.")
    sections = [("Install apps", menu_apps), ("Tweaks", menu_tweaks), ("Cleaner", menu_cleaner), ("Services", menu_services),
                ("Startup", menu_startup), ("Debloat packages", menu_debloat), ("Network / DNS", menu_network), ("Tools", menu_tools)]
    while True:
        print()
        for i, (n, _f) in enumerate(sections, 1):
            print(f"  {i}. {n}")
        c = ask("section (q = quit) > ")
        if c in ("q", "quit", "exit", "b"):
            return
        if c.isdigit() and 1 <= int(c) <= len(sections):
            sections[int(c) - 1][1]()


# ======================================================================================================== graphical interface

LOGO_PNG_B64 = "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"

THEMES = {
    "Dark":      dict(bg="#1e1f22", panel="#2b2d30", fg="#dfe1e5", dim="#8c909a", accent="#3592c4", sel="#2f65ca", ok="#5fb878", warn="#e0a030", err="#e05d5d"),
    "Light":     dict(bg="#f2f2f2", panel="#ffffff", fg="#1e1e1e", dim="#666666", accent="#1f6feb", sel="#b8d4f5", ok="#1a7f37", warn="#9a6700", err="#cf222e"),
    "Manjaro":   dict(bg="#181c1b", panel="#232a28", fg="#e0e6e3", dim="#86908c", accent="#35bf5c", sel="#1f6b3a", ok="#35bf5c", warn="#e0a030", err="#e05d5d"),
    "Nord":      dict(bg="#2e3440", panel="#3b4252", fg="#eceff4", dim="#9aa5b8", accent="#88c0d0", sel="#4c566a", ok="#a3be8c", warn="#ebcb8b", err="#bf616a"),
    "Dracula":   dict(bg="#282a36", panel="#343746", fg="#f8f8f2", dim="#8a8fa8", accent="#bd93f9", sel="#44475a", ok="#50fa7b", warn="#f1fa8c", err="#ff5555"),
    "Gruvbox":   dict(bg="#282828", panel="#3c3836", fg="#ebdbb2", dim="#a89984", accent="#d79921", sel="#504945", ok="#b8bb26", warn="#fabd2f", err="#fb4934"),
    "Solarized": dict(bg="#002b36", panel="#073642", fg="#93a1a1", dim="#657b83", accent="#268bd2", sel="#0b4a5a", ok="#859900", warn="#b58900", err="#dc322f"),
}


def settings_path() -> str:
    return os.path.join(P(real_home()), ".config", "optimaxer", "settings.json")


def load_settings() -> dict:
    try:
        with open(settings_path(), encoding="utf-8") as fh:
            data = json.load(fh)
        return data if isinstance(data, dict) else {}
    except (OSError, ValueError):
        return {}


def save_settings(data: dict) -> None:
    path = settings_path()
    try:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as fh:
            json.dump(data, fh, indent=2)
        if hasattr(os, "geteuid") and os.geteuid() == 0 and not CTX.root:   # keep the file owned by the real user
            import pwd
            u = pwd.getpwnam(real_user())
            os.chown(os.path.dirname(path), u.pw_uid, u.pw_gid)
            os.chown(path, u.pw_uid, u.pw_gid)
    except (OSError, KeyError, ImportError):
        pass


def config_dict(tweak_ids: list, app_pkgs: list) -> dict:
    return {"optimaxer": __version__, "tweaks": sorted(tweak_ids), "apps": sorted(app_pkgs)}


def apply_config(data: dict) -> bool:
    """Apply an exported selection: the tweaks first, then the apps. Same result codes as the command line."""
    ok = True
    for tid in data.get("tweaks", []):
        t = tweak_by_id(tid)
        if not t:
            log(f"Unknown tweak {tid}", "WARN")
            ok = False
        elif not t.apply():
            ok = False
    by_pkg = {a["pkg"]: a for a in APPS}
    for pkg in data.get("apps", []):
        if not install_app(by_pkg.get(pkg) or {"cat": "", "name": pkg, "src": "P", "pkg": pkg}):
            ok = False
    return ok


def desktop_entry() -> str:
    exe = os.path.realpath(sys.argv[0]) if sys.argv and sys.argv[0] else "optimaxer"
    return ("[Desktop Entry]\nType=Application\nName=Optimaxer\nComment=Tweaks, apps and cleanup for Manjaro / Arch\n"
            f"Exec={sys.executable} {exe} gui\nIcon=optimaxer\nTerminal=false\nCategories=System;Settings;\n")


def create_launcher() -> list:
    """Write an application-menu entry (and a copy on the Desktop if there is one). Returns the paths written."""
    import base64
    home = P(real_home())
    made = []
    icon_dir = os.path.join(home, ".local", "share", "icons", "hicolor", "64x64", "apps")
    os.makedirs(icon_dir, exist_ok=True)
    with open(os.path.join(icon_dir, "optimaxer.png"), "wb") as fh:
        fh.write(base64.b64decode(LOGO_PNG_B64))
    for folder in (os.path.join(home, ".local", "share", "applications"), os.path.join(home, "Desktop")):
        if folder.endswith("Desktop") and not os.path.isdir(folder):
            continue
        os.makedirs(folder, exist_ok=True)
        path = os.path.join(folder, "optimaxer.desktop")
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(desktop_entry())
        os.chmod(path, 0o755)
        made.append(path)
    if hasattr(os, "geteuid") and os.geteuid() == 0 and not CTX.root:
        try:
            import pwd
            u = pwd.getpwnam(real_user())
            for path in made + [os.path.join(icon_dir, "optimaxer.png")]:
                os.chown(path, u.pw_uid, u.pw_gid)
        except (KeyError, ImportError, OSError):
            pass
    return made


def tk_available() -> bool:
    try:
        import tkinter  # noqa: F401
        return True
    except ImportError:
        return False


class Gui:
    """Tk window with the same sections as the menu. Long work runs in a thread; log() lines go to the log pane."""

    SECTIONS = ("Home", "Install", "Tweaks", "Cleaner", "Services", "Startup", "Debloat", "Network", "Tools", "Config", "Appearance")

    def __init__(self, root, sync: bool = False):
        import queue
        import threading
        import tkinter as tk
        from tkinter import ttk
        self.tk, self.ttk, self.root, self.sync = tk, ttk, root, sync
        self._queue, self._threading = queue.Queue(), threading
        self.busy = False
        self.settings = load_settings()
        self.rows = {}                      # page -> backing rows
        self.lists = {}                     # page -> Checklist
        self.loaded = set()
        self.theme_name = self.settings.get("theme", "Dark") if self.settings.get("theme") in THEMES else "Dark"
        root.title(f"Optimaxer {__version__}")
        root.geometry("1040x700")
        root.minsize(820, 560)
        try:
            self._icon = tk.PhotoImage(data=LOGO_PNG_B64)
            root.iconphoto(True, self._icon)
        except tk.TclError:
            pass
        self.style = ttk.Style(root)
        try:
            self.style.theme_use("clam")
        except tk.TclError:
            pass
        self._build()
        self.apply_theme(self.theme_name)
        CTX.log_hook = lambda line, level: self._queue.put(("log", line, level))
        CTX.assume_yes = True               # the window asks its own questions
        root.after(60, self._pump)
        self.refresh("Home")

    # ---- layout
    def _build(self):
        tk, ttk = self.tk, self.ttk
        top = ttk.Frame(self.root)
        top.pack(side="top", fill="x")
        ttk.Label(top, text="Optimaxer", style="Title.TLabel").pack(side="left", padx=(12, 8), pady=8)
        self.distro_lbl = ttk.Label(top, text=distro().get("PRETTY_NAME", "Linux"), style="Dim.TLabel")
        self.distro_lbl.pack(side="left")
        self.dry_var = tk.BooleanVar(value=CTX.dry_run)
        ttk.Checkbutton(top, text="Dry run (change nothing)", variable=self.dry_var, command=self._dry_toggled).pack(side="right", padx=12)

        self.paned = ttk.PanedWindow(self.root, orient="vertical")
        self.paned.pack(fill="both", expand=True)
        self.nb = ttk.Notebook(self.paned)
        self.paned.add(self.nb, weight=4)
        self.pages = {}
        for name in self.SECTIONS:
            f = ttk.Frame(self.nb, padding=8)
            self.pages[name] = f
            self.nb.add(f, text=f"  {name}  ")
            getattr(self, f"_page_{name.lower()}")(f)
        self.nb.bind("<<NotebookTabChanged>>", lambda _e: self.refresh(self.current()))

        logf = ttk.Frame(self.paned)
        self.paned.add(logf, weight=1)
        self.log_text = tk.Text(logf, height=7, wrap="word", state="disabled", relief="flat", borderwidth=0, font=("monospace", 9))
        sb = ttk.Scrollbar(logf, command=self.log_text.yview)
        self.log_text.configure(yscrollcommand=sb.set)
        sb.pack(side="right", fill="y")
        self.log_text.pack(side="left", fill="both", expand=True)
        for lvl in ("OK", "WARN", "ERROR", "DRY"):
            self.log_text.tag_configure(lvl)

        bar = ttk.Frame(self.root)
        bar.pack(side="bottom", fill="x")
        self.status = ttk.Label(bar, text="Ready", style="Dim.TLabel")
        self.status.pack(side="left", padx=12, pady=4)
        self.progress = ttk.Progressbar(bar, mode="indeterminate", length=160)
        self.progress.pack(side="right", padx=12)

    def current(self) -> str:
        return self.SECTIONS[self.nb.index(self.nb.select())]

    # ---- theme
    def apply_theme(self, name: str):
        c = THEMES[name]
        self.theme_name, self.colors = name, c
        s = self.style
        self.root.configure(background=c["bg"])
        s.configure(".", background=c["bg"], foreground=c["fg"], fieldbackground=c["panel"], bordercolor=c["panel"], lightcolor=c["panel"],
                    darkcolor=c["panel"], troughcolor=c["panel"], focuscolor=c["accent"], font=("sans-serif", 10))
        s.configure("TLabel", background=c["bg"], foreground=c["fg"])
        s.configure("Title.TLabel", font=("sans-serif", 15, "bold"), foreground=c["accent"])
        s.configure("Dim.TLabel", foreground=c["dim"])
        s.configure("TButton", background=c["panel"], foreground=c["fg"], padding=(10, 5), borderwidth=1)
        s.map("TButton", background=[("active", c["sel"]), ("disabled", c["bg"])], foreground=[("disabled", c["dim"])])
        s.configure("Accent.TButton", background=c["accent"], foreground="#ffffff" if name != "Light" else "#ffffff")
        s.map("Accent.TButton", background=[("active", c["sel"]), ("disabled", c["bg"])])
        s.configure("TCheckbutton", background=c["bg"], foreground=c["fg"])
        s.map("TCheckbutton", background=[("active", c["bg"])])
        s.configure("TNotebook", background=c["bg"], borderwidth=0)
        s.configure("TNotebook.Tab", background=c["panel"], foreground=c["dim"], padding=(6, 4))
        s.map("TNotebook.Tab", background=[("selected", c["bg"])], foreground=[("selected", c["accent"])])
        s.configure("Treeview", background=c["panel"], foreground=c["fg"], fieldbackground=c["panel"], rowheight=24, borderwidth=0)
        s.map("Treeview", background=[("selected", c["sel"])], foreground=[("selected", c["fg"])])
        s.configure("Treeview.Heading", background=c["bg"], foreground=c["dim"], relief="flat")
        s.configure("TEntry", fieldbackground=c["panel"], foreground=c["fg"])
        s.configure("TCombobox", fieldbackground=c["panel"], foreground=c["fg"], background=c["panel"], arrowcolor=c["fg"])
        s.map("TCombobox", fieldbackground=[("readonly", c["panel"])], foreground=[("readonly", c["fg"])],
              selectbackground=[("readonly", c["panel"])], selectforeground=[("readonly", c["fg"])])
        self.root.option_add("*TCombobox*Listbox.background", c["panel"])
        self.root.option_add("*TCombobox*Listbox.foreground", c["fg"])
        self.root.option_add("*TCombobox*Listbox.selectBackground", c["sel"])
        s.configure("TProgressbar", background=c["accent"], troughcolor=c["panel"])
        s.configure("TPanedwindow", background=c["bg"])
        s.configure("TFrame", background=c["bg"])
        s.configure("TScrollbar", background=c["panel"], arrowcolor=c["dim"])
        self.log_text.configure(background=c["panel"], foreground=c["fg"], insertbackground=c["fg"])
        for lvl, key in (("OK", "ok"), ("WARN", "warn"), ("ERROR", "err"), ("DRY", "dim")):
            self.log_text.tag_configure(lvl, foreground=c[key])

    # ---- background work
    def _pump(self):
        try:
            while True:
                item = self._queue.get_nowait()
                if item[0] == "log":
                    self._append_log(item[1], item[2])
                elif item[0] == "call":
                    item[1]()
        except Exception:      # queue.Empty
            pass
        try:
            self.root.after(60, self._pump)
        except self.tk.TclError:
            pass

    def drain(self):
        """Run everything queued so far (used by tests and sync mode)."""
        import queue
        while True:
            try:
                item = self._queue.get_nowait()
            except queue.Empty:
                return
            if item[0] == "log":
                self._append_log(item[1], item[2])
            else:
                item[1]()

    def _append_log(self, line, level):
        t = self.log_text
        t.configure(state="normal")
        t.insert("end", line + "\n", level if level in ("OK", "WARN", "ERROR", "DRY") else ())
        t.see("end")
        t.configure(state="disabled")

    def work(self, label, fn, done=None):
        """Run fn off the UI thread. done(result) runs back on the UI thread."""
        if self.busy:
            self.say("Another task is still running.")
            return
        self.busy = True
        self.status.configure(text=label + "...")
        self.progress.start(12)

        def finish(result, err):
            self.busy = False
            self.progress.stop()
            self.status.configure(text="Ready" if not err else f"Failed: {err}")
            if err:
                log(f"{label}: {err}", "ERROR")
            elif done:
                done(result)

        def body():
            res, err = None, None
            try:
                res = fn()
            except Exception as exc:      # keep the window alive; the cause goes to the log pane
                err = f"{type(exc).__name__}: {exc}"
            self._queue.put(("call", lambda: finish(res, err)))

        if self.sync:
            body()
            self.drain()
        else:
            self._threading.Thread(target=body, daemon=True).start()

    def say(self, text):
        self.status.configure(text=text)
        log(text, "INFO")

    def ask_yes(self, question) -> bool:
        if self.sync:
            return True
        from tkinter import messagebox
        return messagebox.askyesno("Optimaxer", question, parent=self.root)

    def _dry_toggled(self):
        CTX.dry_run = bool(self.dry_var.get())
        self.say("Dry run on: commands are shown, nothing changes." if CTX.dry_run else "Dry run off: changes are real.")

    # ---- checklist widget
    def checklist(self, parent, key, columns, with_filter=True, checks=True):
        """Treeview with a check column, filter box and select all/none. Rows are set with self.fill(key, rows)."""
        tk, ttk = self.tk, self.ttk
        frame = ttk.Frame(parent)
        bar = ttk.Frame(frame)
        bar.pack(fill="x", pady=(0, 4))
        fv = tk.StringVar()
        if with_filter:
            ttk.Label(bar, text="Filter").pack(side="left")
            e = ttk.Entry(bar, textvariable=fv, width=26)
            e.pack(side="left", padx=6)
        cols = (["chk"] if checks else []) + [c[0] for c in columns]
        tree = ttk.Treeview(frame, columns=cols, show="headings", selectmode="browse" if not checks else "extended")
        if checks:
            tree.heading("chk", text="")
            tree.column("chk", width=34, stretch=False, anchor="center")
        for cid, head, width in columns:
            tree.heading(cid, text=head)
            tree.column(cid, width=width, anchor="w", stretch=(cid == columns[-1][0]))
        vs = ttk.Scrollbar(frame, orient="vertical", command=tree.yview)
        tree.configure(yscrollcommand=vs.set)
        info = {"frame": frame, "tree": tree, "cols": cols, "filter": fv, "checked": set(), "data": [], "checks": checks, "bar": bar, "iids": {}}
        if checks:
            ttk.Button(bar, text="Select all", command=lambda: self.check_all(key, True)).pack(side="right")
            ttk.Button(bar, text="Select none", command=lambda: self.check_all(key, False)).pack(side="right", padx=6)
            tree.bind("<Button-1>", lambda ev: self._click(key, ev))
            tree.bind("<space>", lambda ev: self._space(key))
        fv.trace_add("write", lambda *_: self.render(key))
        vs.pack(side="right", fill="y")
        tree.pack(side="left", fill="both", expand=True)
        self.lists[key] = info
        return frame

    def fill(self, key, data, keep=True):
        """data: list of (id, tuple_of_values, tag). Keeps the ticks of rows that still exist."""
        info = self.lists[key]
        ids = {d[0] for d in data}
        info["checked"] = (info["checked"] & ids) if keep else set()
        info["data"] = data
        self.render(key)

    def render(self, key):
        info = self.lists[key]
        tree, flt = info["tree"], info["filter"].get().strip().lower()
        tree.delete(*tree.get_children())
        info["iids"] = {}
        for rid, vals, tag in info["data"]:
            if flt and flt not in " ".join(str(v) for v in vals).lower():
                continue
            row = ((("☑" if rid in info["checked"] else "☐"),) if info["checks"] else ()) + tuple(vals)
            iid = tree.insert("", "end", values=row, tags=(tag,) if tag else ())
            info["iids"][iid] = rid
        c = getattr(self, "colors", THEMES["Dark"])
        tree.tag_configure("on", foreground=c["ok"])
        tree.tag_configure("warn", foreground=c["warn"])
        tree.tag_configure("dim", foreground=c["dim"])

    def _toggle(self, key, rid):
        info = self.lists[key]
        info["checked"] ^= {rid}
        for iid, r in info["iids"].items():
            if r == rid:
                info["tree"].set(iid, "chk", "☑" if rid in info["checked"] else "☐")

    def _click(self, key, ev):
        tree = self.lists[key]["tree"]
        if tree.identify_region(ev.x, ev.y) == "cell" and tree.identify_column(ev.x) == "#1":
            iid = tree.identify_row(ev.y)
            if iid:
                self._toggle(key, self.lists[key]["iids"][iid])
                return "break"

    def _space(self, key):
        info = self.lists[key]
        for iid in info["tree"].selection():
            self._toggle(key, info["iids"][iid])
        return "break"

    def check_all(self, key, on):
        info = self.lists[key]
        shown = set(info["iids"].values())
        info["checked"] = (info["checked"] | shown) if on else (info["checked"] - shown)
        self.render(key)

    def ticked(self, key) -> list:
        info = self.lists[key]
        return [d[0] for d in info["data"] if d[0] in info["checked"]]

    def tick(self, key, ids):
        info = self.lists[key]
        info["checked"] = {i for i in ids if any(d[0] == i for d in info["data"])}
        self.render(key)

    # ---- pages
    def _buttons(self, parent, specs):
        ttk = self.ttk
        bar = ttk.Frame(parent)
        bar.pack(fill="x", pady=(6, 0))
        for text, cmd, accent in specs:
            ttk.Button(bar, text=text, command=cmd, style="Accent.TButton" if accent else "TButton").pack(side="left", padx=(0, 6))
        return bar

    def _page_home(self, f):
        ttk = self.ttk
        ttk.Label(f, text="This system", style="Title.TLabel").pack(anchor="w")
        self.summary = ttk.Label(f, text="", style="Dim.TLabel", justify="left")
        self.summary.pack(anchor="w", pady=(2, 8))
        self.checklist(f, "home", [("k", "Item", 160), ("v", "Value", 600)], with_filter=False, checks=False).pack(fill="both", expand=True)
        self._buttons(f, [("Apply the Safe preset", lambda: self.run_tweaks(preset_ids("safe"), True), True),
                          ("Update the system", lambda: self.work("Updating the system", tool_update_system, lambda _r: self.refresh("Home")), False),
                          ("Undo everything", self.undo_everything, False)])

    def _page_install(self, f):
        self.checklist(f, "apps", [("name", "App", 220), ("cat", "Category", 120), ("src", "Source", 70), ("pkg", "Package", 190), ("st", "Status", 90)]).pack(fill="both", expand=True)
        self._buttons(f, [("Install selected", lambda: self.run_apps(True), True), ("Remove selected", lambda: self.run_apps(False), False),
                          ("Upgrade everything", lambda: self.work("Updating the system", tool_update_system), False), ("Refresh", lambda: self.refresh("Install", True), False)])

    def _page_tweaks(self, f):
        ttk, tk = self.ttk, self.tk
        bar = ttk.Frame(f)
        bar.pack(fill="x", pady=(0, 6))
        ttk.Label(bar, text="Preset").pack(side="left")
        self.preset_var = tk.StringVar(value=PRESETS[0])
        ttk.Combobox(bar, textvariable=self.preset_var, values=list(PRESETS), state="readonly", width=10).pack(side="left", padx=6)
        ttk.Button(bar, text="Select preset", command=self.select_preset).pack(side="left")
        self.checklist(f, "tweaks", [("name", "Tweak", 300), ("cat", "Category", 110), ("risk", "Risk", 70), ("st", "Status", 80), ("desc", "What it does", 500)]).pack(fill="both", expand=True)
        self.tweak_info = ttk.Label(f, text="", style="Dim.TLabel", wraplength=900, justify="left")
        self.tweak_info.pack(anchor="w", pady=(4, 0))
        self.lists["tweaks"]["tree"].bind("<<TreeviewSelect>>", self._tweak_selected)
        self._buttons(f, [("Apply selected", lambda: self.run_tweaks(self.ticked("tweaks"), True), True), ("Undo selected", lambda: self.run_tweaks(self.ticked("tweaks"), False), False),
                          ("Refresh", lambda: self.refresh("Tweaks", True), False)])

    def _page_cleaner(self, f):
        ttk = self.ttk
        self.clean_total = ttk.Label(f, text="", style="Dim.TLabel")
        self.clean_total.pack(anchor="w")
        self.checklist(f, "clean", [("name", "Item", 360), ("risk", "Risk", 80), ("size", "Size", 110)], with_filter=False).pack(fill="both", expand=True)
        self._buttons(f, [("Clean selected", self.run_clean, True), ("Scan again", lambda: self.refresh("Cleaner", True), False)])

    def _page_services(self, f):
        self.checklist(f, "services", [("unit", "Service", 240), ("st", "State", 80), ("run", "Running", 80), ("note", "Notes", 520)]).pack(fill="both", expand=True)
        self._buttons(f, [("Disable selected", lambda: self.run_services("disable"), True), ("Enable selected", lambda: self.run_services("enable"), False),
                          ("Restore saved state", lambda: self.run_services("restore"), False)])

    def _page_startup(self, f):
        self.checklist(f, "startup", [("name", "Entry", 240), ("scope", "Scope", 70), ("st", "State", 80), ("exec", "Command", 520)]).pack(fill="both", expand=True)
        self._buttons(f, [("Disable selected", lambda: self.run_startup(False), True), ("Enable selected", lambda: self.run_startup(True), False)])

    def _page_debloat(self, f):
        ttk = self.ttk
        ttk.Label(f, text="Optional packages you may not need. Nothing is selected for you.", style="Dim.TLabel").pack(anchor="w")
        self.checklist(f, "debloat", [("label", "Package", 300), ("pkg", "Name", 180), ("size", "Size", 100)], with_filter=False).pack(fill="both", expand=True)
        self._buttons(f, [("Remove selected", self.run_debloat, True)])

    def _page_network(self, f):
        ttk = self.ttk
        ttk.Label(f, text="DNS provider. Applies to the active NetworkManager connections; Restore returns to the saved settings.", style="Dim.TLabel").pack(anchor="w")
        self.checklist(f, "dns", [("name", "Provider", 220), ("addr", "Servers", 480)], with_filter=False, checks=False).pack(fill="both", expand=True)
        self._buttons(f, [("Use selected provider", self.run_dns, True), ("Restore previous DNS", lambda: self.work("Restoring DNS", restore_dns), False)])

    def _page_tools(self, f):
        ttk = self.ttk
        ttk.Label(f, text="Results appear in the log pane below.", style="Dim.TLabel").pack(anchor="w", pady=(0, 6))

        def show(lines):
            for l in lines:
                log(l, "INFO")
        tools = [("System information", lambda: self.work("System information", sysinfo, lambda r: show(f"{k:<10} {v}" for k, v in r))),
                 ("Update the system (pacman -Syu)", lambda: self.work("Updating the system", tool_update_system)),
                 ("Rank mirrors (Manjaro fasttrack)", lambda: self.work("Ranking mirrors", tool_rank_mirrors)),
                 ("Show failed systemd units", lambda: self.work("Checking units", tool_failed_units, lambda r: show(r or ["No failed units."]))),
                 ("Find .pacnew / .pacsave files", lambda: self.work("Searching", tool_pacnew, lambda r: show(r or ["None found."]))),
                 ("Disk health (SMART)", lambda: self.work("Reading SMART data", tool_smart, lambda r: show(r))),
                 ("Check for an Optimaxer update", lambda: self.work("Checking for updates", self_update))]
        for text, cmd in tools:
            ttk.Button(f, text=text, command=cmd, width=36).pack(anchor="w", pady=2)

    def _page_config(self, f):
        ttk = self.ttk
        ttk.Label(f, text="Export the tweaks and apps you ticked, load them on another machine, or add Optimaxer to the application menu.", style="Dim.TLabel",
                  wraplength=900, justify="left").pack(anchor="w", pady=(0, 8))
        for text, cmd in (("Export ticked tweaks and apps...", self.export_config), ("Import a saved selection...", self.import_config),
                          ("Create application menu / desktop launcher", self.make_launcher)):
            ttk.Button(f, text=text, command=cmd, width=40).pack(anchor="w", pady=3)
        ttk.Label(f, text="Unattended: optimaxer tweaks apply --preset safe     optimaxer config apply my-setup.json", style="Dim.TLabel").pack(anchor="w", pady=(14, 0))

    def _page_appearance(self, f):
        ttk, tk = self.ttk, self.tk
        ttk.Label(f, text="Theme").pack(anchor="w")
        self.theme_var = tk.StringVar(value=self.theme_name)
        cb = ttk.Combobox(f, textvariable=self.theme_var, values=list(THEMES), state="readonly", width=18)
        cb.pack(anchor="w", pady=6)
        cb.bind("<<ComboboxSelected>>", lambda _e: self.set_theme(self.theme_var.get()))
        ttk.Label(f, text=f"Optimaxer {__version__}  -  github.com/bliper2/optimaxer  -  MIT license", style="Dim.TLabel").pack(anchor="w", pady=(16, 0))

    def set_theme(self, name):
        if name in THEMES:
            self.apply_theme(name)
            self.settings["theme"] = name
            save_settings(self.settings)
            for k in self.lists:
                self.render(k)

    # ---- loading
    def refresh(self, page, force=False):
        if page in self.loaded and not force:
            return
        loader = {"Home": self.load_home, "Install": self.load_apps, "Tweaks": self.load_tweaks, "Cleaner": self.load_clean, "Services": self.load_services,
                  "Startup": self.load_startup, "Debloat": self.load_debloat, "Network": self.load_dns}.get(page)
        if not loader:
            return
        self.loaded.add(page)
        compute, show = loader()
        self.work(f"Reading {page.lower()}", compute, show)

    def reload(self, *pages):
        for p in pages:
            self.loaded.discard(p)
        self.refresh(self.current())

    def load_home(self):
        def compute():
            state = load_state()
            return sysinfo(), len([k for k in state if tweak_by_id(k)]), len([t for t in TWEAKS if t.available()]), tool_failed_units()

        def show(r):
            info, applied, total, failed = r
            self.fill("home", [(k, (k, v), None) for k, v in info], keep=False)
            self.summary.configure(text=f"{applied} of {total} tweaks applied    {len(failed)} failed systemd unit(s)")
        return compute, show

    def load_apps(self):
        src = {"P": "Repo", "A": "AUR", "F": "Flatpak"}
        return (lambda: {a["pkg"]: app_installed(a) for a in APPS},
                lambda r: self.fill("apps", [(a["pkg"], (a["name"], a["cat"], src.get(a["src"], a["src"]), a["pkg"], "Installed" if r[a["pkg"]] else ""), "on" if r[a["pkg"]] else None)
                                             for a in APPS]))

    def load_tweaks(self):
        def compute():
            rows = [t for t in TWEAKS if t.available()]
            return [(t, t.is_applied()) for t in rows]
        return compute, lambda r: self.fill("tweaks", [(t.id, (t.name, t.cat, t.risk, "Applied" if ap else "", t.desc), "on" if ap else ("warn" if t.risk != "safe" else None)) for t, ap in r])

    def load_clean(self):
        def show(r):
            self.fill("clean", [(t["id"], (t["name"], t["risk"], "on demand" if r[t["id"]] < 0 else fmt_size(r[t["id"]])), None) for t in CLEAN_TARGETS], keep=False)
            self.clean_total.configure(text=f"Reclaimable: {fmt_size(sum(v for v in r.values() if v > 0))}")
        return (lambda: {t["id"]: target_size(t) for t in CLEAN_TARGETS}), show

    def load_services(self):
        return service_rows, lambda r: self.fill("services", [(x["unit"], (x["unit"], "enabled" if x["enabled"] else "disabled", "yes" if x["active"] else "", f"{x['label']}: {x['hint']}"), None) for x in r])

    def load_startup(self):
        def show(r):
            self.rows["startup"] = {f"{x['scope']}:{x['name']}": x for x in r}
            self.fill("startup", [(f"{x['scope']}:{x['name']}", (x["name"], x["scope"], "enabled" if x["enabled"] else "disabled", x["exec"]), None if x["enabled"] else "dim") for x in r])
        return autostart_items, show

    def load_debloat(self):
        return installed_debloat, lambda r: self.fill("debloat", [(x["pkg"], (x["label"], x["pkg"], fmt_size(x["size"])), None) for x in r])

    def load_dns(self):
        def show(_r):
            self.fill("dns", [(k, (DNS[k][0], ", ".join(DNS[k][1]) or "router default"), None) for k in DNS], keep=False)
        return (lambda: None), show

    # ---- actions
    def select_preset(self):
        self.tick("tweaks", preset_ids(self.preset_var.get()))
        self.say(f"Preset '{self.preset_var.get()}' selects {len(self.ticked('tweaks'))} tweaks. Press Apply selected to run them.")

    def _tweak_selected(self, _e):
        info = self.lists["tweaks"]
        sel = info["tree"].selection()
        t = tweak_by_id(info["iids"].get(sel[0])) if sel else None
        self.tweak_info.configure(text=f"{t.name} [{t.risk}]: {t.desc}" if t else "")

    def run_tweaks(self, ids, apply):
        if not ids:
            return self.say("Nothing selected.")
        if not self.ask_yes(f"{'Apply' if apply else 'Undo'} {len(ids)} tweak(s)?"):
            return

        def go():
            res = [(tweak_by_id(i).apply() if apply else tweak_by_id(i).undo()) for i in ids if tweak_by_id(i)]
            log(f"{'Applied' if apply else 'Undone'}: {sum(res)} ok, {len(res) - sum(res)} failed", "OK" if all(res) else "WARN")
        self.work("Applying tweaks" if apply else "Undoing tweaks", go, lambda _r: self.reload("Tweaks", "Home"))

    def run_apps(self, install):
        by = {a["pkg"]: a for a in APPS}
        chosen = [by[i] for i in self.ticked("apps")]
        if not chosen:
            return self.say("Nothing selected.")
        if not self.ask_yes(f"{'Install' if install else 'Remove'} {len(chosen)} app(s)?"):
            return

        def go():
            res = [install_app(a) if install else remove_app(a) for a in chosen]
            log(f"{'Install' if install else 'Remove'} finished: {sum(res)} ok, {len(res) - sum(res)} failed", "OK" if all(res) else "WARN")
        self.work("Installing" if install else "Removing", go, lambda _r: self.reload("Install"))

    def run_clean(self):
        ids = self.ticked("clean")
        if not ids:
            return self.say("Nothing selected.")
        if not self.ask_yes(f"Clean {len(ids)} item(s)? This cannot be undone."):
            return
        by = {t["id"]: t for t in CLEAN_TARGETS}

        def go():
            log(f"Total freed: {fmt_size(sum(clean_target(by[i]) for i in ids))}", "OK")
        self.work("Cleaning", go, lambda _r: self.reload("Cleaner"))

    def run_services(self, what):
        units = self.ticked("services")
        if not units:
            return self.say("Nothing selected.")
        if not self.ask_yes(f"{what.capitalize()} {len(units)} service(s)?"):
            return

        def go():
            for u in units:
                {"enable": lambda: set_service(u, True), "disable": lambda: set_service(u, False), "restore": lambda: restore_service(u)}[what]()
        self.work("Changing services", go, lambda _r: self.reload("Services"))

    def run_startup(self, enable):
        keys = self.ticked("startup")
        if not keys:
            return self.say("Nothing selected.")

        def go():
            for k in keys:
                set_autostart(self.rows["startup"][k], enable)
        self.work("Changing startup entries", go, lambda _r: self.reload("Startup"))

    def run_debloat(self):
        pkgs = self.ticked("debloat")
        if not pkgs:
            return self.say("Nothing selected.")
        if not self.ask_yes(f"Remove {len(pkgs)} package(s) with pacman -Rns?"):
            return

        def go():
            for p in pkgs:
                ok = run(["pacman", "-Rns", "--noconfirm", p], write=True)[0] == 0
                log(f"{p}: {'removed' if ok else 'not removed (other packages depend on it?)'}", "OK" if ok else "WARN")
        self.work("Removing packages", go, lambda _r: self.reload("Debloat"))

    def run_dns(self):
        info = self.lists["dns"]
        sel = info["tree"].selection()
        if not sel:
            return self.say("Pick a provider first.")
        key = info["iids"][sel[0]]
        self.work("Setting DNS", lambda: set_dns(key))

    def undo_everything(self):
        if self.ask_yes("Undo every tweak, service change and DNS change Optimaxer made?"):
            self.work("Undoing everything", undo_all, lambda _r: self.reload("Tweaks", "Services", "Home"))

    def export_config(self):
        data = config_dict(self.ticked("tweaks"), self.ticked("apps"))
        if self.sync:
            path = self.settings.get("_test_path")
        else:
            from tkinter import filedialog
            path = filedialog.asksaveasfilename(parent=self.root, defaultextension=".json", initialfile="optimaxer-setup.json", filetypes=[("JSON", "*.json")])
        if path:
            with open(path, "w", encoding="utf-8") as fh:
                json.dump(data, fh, indent=2)
            self.say(f"Saved {len(data['tweaks'])} tweaks and {len(data['apps'])} apps to {path}")

    def import_config(self):
        if self.sync:
            path = self.settings.get("_test_path")
        else:
            from tkinter import filedialog
            path = filedialog.askopenfilename(parent=self.root, filetypes=[("JSON", "*.json")])
        if not path:
            return
        try:
            with open(path, encoding="utf-8") as fh:
                data = json.load(fh)
        except (OSError, ValueError) as exc:
            return self.say(f"Could not read that file: {exc}")
        self.refresh("Tweaks")
        self.refresh("Install")
        self.tick("tweaks", data.get("tweaks", []))
        self.tick("apps", data.get("apps", []))
        self.say(f"Selected {len(self.ticked('tweaks'))} tweaks and {len(self.ticked('apps'))} apps. Review them in the Tweaks and Install tabs, then apply.")

    def make_launcher(self):
        try:
            made = create_launcher()
        except OSError as exc:
            return self.say(f"Could not create the launcher: {exc}")
        self.say("Launcher created: " + ", ".join(made))


def run_gui() -> int:
    if not tk_available():
        print("The graphical interface needs Tk. Install it with:  sudo pacman -S tk", file=sys.stderr)
        return 2
    import tkinter as tk
    try:
        root = tk.Tk()
    except tk.TclError as exc:
        print(f"No display available ({exc}). Use the text menu: optimaxer", file=sys.stderr)
        return 2
    CTX.quiet = True
    Gui(root)
    root.mainloop()
    return 0


def ensure_root_gui(argv: list) -> None:
    """A window must be opened as the user, then elevated with the display variables kept. pkexec drops them, so pass them back."""
    if CTX.root or CTX.dry_run or not hasattr(os, "geteuid") or os.geteuid() == 0:
        return
    keep = [f"{k}={os.environ[k]}" for k in ("DISPLAY", "XAUTHORITY", "WAYLAND_DISPLAY", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS") if os.environ.get(k)]
    keep.append("SUDO_USER=" + (os.environ.get("USER") or "root"))
    script = os.path.realpath(sys.argv[0])
    if have("pkexec"):
        print("Optimaxer needs administrator rights; asking for your password...")
        os.execvp("pkexec", ["pkexec", "env"] + keep + [sys.executable, script] + argv)
    if have("sudo"):
        os.execvp("sudo", ["sudo", "-E", "env"] + keep + [sys.executable, script] + argv)
    sys.exit("Optimaxer needs root: install polkit (pkexec) or sudo, or tick Dry run by starting with --dry-run.")


# ======================================================================================================== command line

def ensure_root(argv: list) -> None:
    if CTX.root or CTX.dry_run or not hasattr(os, "geteuid") or os.geteuid() == 0:
        return
    if not have("sudo"):
        sys.exit("Optimaxer needs root: run it with sudo (or use --dry-run).")
    print("Optimaxer needs administrator rights for system changes; asking sudo...")
    os.execvp("sudo", ["sudo", "-E", sys.executable, os.path.realpath(sys.argv[0])] + argv)


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="optimaxer", description="Optimaxer for Manjaro/Arch Linux")
    p.add_argument("--dry-run", action="store_true", help="show what would change without changing anything")
    p.add_argument("-y", "--yes", action="store_true", help="do not ask for confirmation")
    p.add_argument("--version", action="version", version=f"optimaxer {__version__}")
    sub = p.add_subparsers(dest="cmd")
    t = sub.add_parser("tweaks", help="list/apply/undo tweaks")
    t.add_argument("action", choices=["list", "apply", "undo"])
    t.add_argument("ids", nargs="*")
    t.add_argument("--preset", choices=PRESETS)
    t.add_argument("--all", action="store_true", help="with undo: every applied tweak")
    a = sub.add_parser("apps", help="list/install/remove apps")
    a.add_argument("action", choices=["list", "install", "remove"])
    a.add_argument("names", nargs="*", help="package names (e.g. firefox brave-bin)")
    c = sub.add_parser("clean", help="scan or clean")
    c.add_argument("action", choices=["scan", "run"])
    c.add_argument("ids", nargs="*")
    sub.add_parser("services", help="list notable services")
    sub.add_parser("startup", help="list autostart entries")
    sub.add_parser("debloat", help="list removable optional packages")
    d = sub.add_parser("dns", help="set DNS provider")
    d.add_argument("provider", choices=list(DNS) + ["restore"])
    cf = sub.add_parser("config", help="apply a saved selection (exported from the GUI)")
    cf.add_argument("action", choices=["apply"])
    cf.add_argument("file")
    sub.add_parser("launcher", help="add Optimaxer to the application menu (and Desktop)")
    sub.add_parser("gui", help="open the graphical interface (needs Tk: sudo pacman -S tk)")
    sub.add_parser("update", help="update Optimaxer itself")
    sub.add_parser("info", help="system information")
    return p


def main(argv: list | None = None) -> int:
    argv = sys.argv[1:] if argv is None else argv
    args = build_parser().parse_args(argv)
    CTX.dry_run, CTX.assume_yes = args.dry_run, args.yes
    if not is_arch_family() and not CTX.runner:
        print("This edition targets Manjaro and other Arch-based distributions (pacman).", file=sys.stderr)
        return 2
    mutating = args.cmd in ("tweaks", "apps", "clean", "dns", "config", None) and not (
        (args.cmd == "tweaks" and args.action == "list") or (args.cmd == "apps" and args.action == "list") or (args.cmd == "clean" and args.action == "scan"))
    if args.cmd == "gui":
        if not tk_available() and not CTX.dry_run and hasattr(os, "geteuid") and os.geteuid() != 0 and have("pkexec"):
            print("Tk is missing. Install it with:  sudo pacman -S tk", file=sys.stderr)
            return 2
        ensure_root_gui(argv)
    elif mutating:
        ensure_root(argv)
    rc = 0
    if args.cmd == "gui":
        rc = run_gui()
    elif args.cmd == "launcher":
        for path in create_launcher():
            print("Created", path)
    elif args.cmd == "config":
        try:
            with open(args.file, encoding="utf-8") as fh:
                rc = 0 if apply_config(json.load(fh)) else 1
        except (OSError, ValueError) as exc:
            print(f"Cannot read {args.file}: {exc}", file=sys.stderr)
            rc = 2
    elif args.cmd is None:
        interactive()
    elif args.cmd == "info":
        for k, v in sysinfo():
            print(f"{k:<10} {v}")
    elif args.cmd == "tweaks":
        if args.action == "list":
            for t in TWEAKS:
                if t.available():
                    print(f"{'*' if t.is_applied() else ' '} {t.id:<18} [{t.risk}] {t.name}")
        else:
            ids = list(args.ids)
            if args.preset:
                ids += preset_ids(args.preset)
            if args.action == "undo" and args.all:
                ids = [k for k in load_state() if tweak_by_id(k)]
            if not ids:
                print("No tweaks given: pass ids, --preset, or --all (undo).", file=sys.stderr)
                return 2
            bad = 0
            for tid in ids:
                t = tweak_by_id(tid)
                if not t:
                    log(f"Unknown tweak {tid}", "WARN")
                    bad += 1
                elif not (t.apply() if args.action == "apply" else t.undo()):
                    bad += 1
            rc = 1 if bad else 0
    elif args.cmd == "apps":
        if args.action == "list":
            for a in APPS:
                print(f"{'*' if app_installed(a) else ' '} {a['pkg']:<28} ({a['src']}) {a['name']}")
        else:
            by_pkg = {a["pkg"]: a for a in APPS}
            res = []
            for n in args.names:
                app = by_pkg.get(n) or {"cat": "", "name": n, "src": "P", "pkg": n}
                res.append(install_app(app) if args.action == "install" else remove_app(app))
            rc = 0 if all(res) else 1
    elif args.cmd == "clean":
        targets = [t for t in CLEAN_TARGETS if not args.ids or t["id"] in args.ids]
        if args.action == "scan":
            for t in targets:
                s = target_size(t)
                print(f"{t['id']:<16} {'on demand' if s < 0 else fmt_size(s):>10}  {t['name']}")
        else:
            if not args.ids:
                targets = [t for t in targets if t["risk"] == "safe"]
            print(f"Freed: {fmt_size(sum(clean_target(t) for t in targets))}")
    elif args.cmd == "services":
        for r in service_rows():
            print(f"{r['unit']:<38} {'enabled ' if r['enabled'] else 'disabled'} {r['label']}")
    elif args.cmd == "startup":
        for r in autostart_items():
            print(f"{'on ' if r['enabled'] else 'off'} {r['scope']:<6} {r['name']}")
    elif args.cmd == "debloat":
        for r in installed_debloat():
            print(f"{r['pkg']:<22} {fmt_size(r['size']):>10}  {r['label']}")
    elif args.cmd == "dns":
        ok = restore_dns() if args.provider == "restore" else set_dns(args.provider)
        rc = 0 if ok else 1
    elif args.cmd == "update":
        rc = 0 if self_update() else 1
    return rc


if __name__ == "__main__":
    sys.exit(main())
