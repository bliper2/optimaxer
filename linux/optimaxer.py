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

__version__ = "2.1.0"
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


CTX = Ctx()


def P(path: str) -> str:
    """Map an absolute system path under the optional test root."""
    return os.path.join(CTX.root, path.lstrip("/")) if CTX.root else path


def log(msg: str, level: str = "INFO") -> None:
    line = f"[{time.strftime('%H:%M:%S')}] {level:<5} {msg}"
    if not CTX.quiet:
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
    mutating = args.cmd in ("tweaks", "apps", "clean", "dns", None) and not (
        (args.cmd == "tweaks" and args.action == "list") or (args.cmd == "apps" and args.action == "list") or (args.cmd == "clean" and args.action == "scan"))
    if mutating:
        ensure_root(argv)
    rc = 0
    if args.cmd is None:
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
