"""Tests for the Linux edition. They run on any OS: commands go to a fake system and files to a temp root."""
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import optimaxer as ox  # noqa: E402


class FakeSys:
    def __init__(self):
        self.pkgs, self.enabled, self.active = set(), set(), set()
        self.units = {"fstrim.timer", "earlyoom.service", "paccache.timer", "avahi-daemon.service", "avahi-daemon.socket",
                      "NetworkManager-wait-online.service", "cups.service", "ufw.service"}
        self.commands, self.have = [], {"pacman", "systemctl", "yay", "flatpak", "nmcli", "paccache", "sudo"}
        self.fail_pkgs, self.nm = set(), {}

    def __call__(self, cmd, user, write, input):
        self.commands.append((list(cmd), user, write))
        c = cmd
        if c[0] == "which":
            return (0, "") if c[1] in self.have else (1, "")
        if c[:2] == ["pacman", "-Q"]:
            return (0, "") if c[2] in self.pkgs else (1, "")
        if c[:2] == ["pacman", "-S"] or c[0] in ("yay", "paru"):
            names = [a for a in c[1:] if not a.startswith("-")]
            if set(names) & self.fail_pkgs:
                return 1, "error: target not found"
            self.pkgs |= set(names)
            return 0, ""
        if c[:2] == ["pacman", "-Rns"]:
            self.pkgs -= set(a for a in c[2:] if not a.startswith("-"))
            return 0, ""
        if c[:2] == ["systemctl", "is-enabled"]:
            return (0, "enabled\n") if c[2] in self.enabled else (1, "disabled\n")
        if c[:2] == ["systemctl", "is-active"]:
            return (0, "active\n") if c[2] in self.active else (3, "inactive\n")
        if c[:2] == ["systemctl", "list-unit-files"]:
            return (0, f"{c[2]} enabled\n") if c[2] in self.units else (0, "")
        if c[:2] == ["systemctl", "enable"]:
            self.enabled.add(c[-1])
            if "--now" in c:
                self.active.add(c[-1])
            return 0, ""
        if c[:2] == ["systemctl", "disable"]:
            self.enabled.discard(c[-1])
            if "--now" in c:
                self.active.discard(c[-1])
            return 0, ""
        if c[:3] == ["flatpak", "install", "-y"]:
            self.pkgs.add(c[-1])
            return 0, ""
        if c[:2] == ["flatpak", "info"]:
            return (0, "") if c[2] in self.pkgs else (1, "")
        if c[:2] == ["nmcli", "-t"]:
            return 0, "Wired connection 1:802-3-ethernet:enp3s0\nlo-ish:loopback:lo\n"
        if c[:2] == ["nmcli", "-g"]:
            return 0, self.nm.get((c[-1], c[2]), "")
        return 0, ""

    def ran(self, *prefix):
        return any(cmd[:len(prefix)] == list(prefix) for cmd, _u, _w in self.commands)


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        ox.CTX.root = self.tmp.name
        ox.CTX.dry_run = False
        ox.CTX.quiet = True
        ox.CTX.assume_yes = True
        ox.CTX.user = "tester"
        ox.CTX.home = "/home/tester"          # a path inside the fake root, like on a real system
        os.makedirs(ox.P(ox.CTX.home))
        self.sys = FakeSys()
        ox.CTX.runner = self.sys
        self.put("/etc/os-release", 'NAME="Manjaro Linux"\nID=manjaro\nID_LIKE=arch\nPRETTY_NAME="Manjaro Linux"\n')

    def tearDown(self):
        ox.CTX.runner = None
        ox.CTX.root = ""
        ox.CTX.home = None
        self.tmp.cleanup()

    def put(self, path, text):
        full = ox.P(path)
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "w", encoding="utf-8") as fh:
            fh.write(text)

    def get(self, path):
        return ox.read_text(path)


class TweakFiles(Base):
    def test_apply_undo_files(self):
        t = ox.tweak_by_id("perf-vm")
        self.assertFalse(t.is_applied())
        self.assertTrue(t.apply())
        self.assertIn("vm.swappiness = 20", self.get("/etc/sysctl.d/99-optimaxer-vm.conf"))
        self.assertTrue(self.sys.ran("sysctl", "--system"))
        self.assertIn("perf-vm", ox.load_state())
        self.assertTrue(t.undo())
        self.assertIsNone(self.get("/etc/sysctl.d/99-optimaxer-vm.conf"))
        self.assertNotIn("perf-vm", ox.load_state())

    def test_existing_file_restored(self):
        self.put("/etc/sysctl.d/99-optimaxer-vm.conf", "vm.swappiness = 60\n")
        t = ox.tweak_by_id("perf-vm")
        t.apply()
        t.undo()
        self.assertEqual(self.get("/etc/sysctl.d/99-optimaxer-vm.conf"), "vm.swappiness = 60\n")

    def test_dry_run_changes_nothing(self):
        ox.CTX.dry_run = True
        ox.tweak_by_id("perf-vm").apply()
        self.assertIsNone(self.get("/etc/sysctl.d/99-optimaxer-vm.conf"))
        self.assertEqual(ox.load_state(), {})
        self.assertFalse(any(w for _c, _u, w in self.sys.commands))

    def test_undo_unknown(self):
        self.assertFalse(ox.tweak_by_id("perf-vm").undo())


class TweakEdits(Base):
    PACMAN = "[options]\nHoldPkg     = pacman glibc\n#Color\n#ParallelDownloads = 5\n\n[core]\nInclude = /etc/pacman.d/mirrorlist\n"

    def test_uncomment_and_restore(self):
        self.put("/etc/pacman.conf", self.PACMAN)
        t = ox.tweak_by_id("pm-pacman")
        self.assertTrue(t.apply())
        text = self.get("/etc/pacman.conf")
        self.assertIn("\nColor\n", text)
        self.assertIn("\nParallelDownloads = 5\n", text)
        self.assertNotIn("#Color", text)
        self.assertTrue(t.undo())
        self.assertEqual(self.get("/etc/pacman.conf"), self.PACMAN)

    def test_insert_into_section_and_remove(self):
        original = "[options]\nHoldPkg = pacman\n\n[core]\nInclude = x\n"
        self.put("/etc/pacman.conf", original)
        t = ox.tweak_by_id("pm-pacman")
        t.apply()
        self.assertTrue(self.get("/etc/pacman.conf").startswith("[options]\nColor\nParallelDownloads") or "ParallelDownloads = 5" in self.get("/etc/pacman.conf"))
        t.undo()
        self.assertEqual(self.get("/etc/pacman.conf"), original)

    def test_already_set_is_untouched_on_undo(self):
        text = "[options]\nColor\nParallelDownloads = 5\n"
        self.put("/etc/pacman.conf", text)
        t = ox.tweak_by_id("pm-pacman")
        t.apply()
        t.undo()
        self.assertEqual(self.get("/etc/pacman.conf"), text)

    def test_grub_timeout(self):
        self.put("/etc/default/grub", "GRUB_DEFAULT=saved\nGRUB_TIMEOUT=5\n")
        t = ox.tweak_by_id("perf-grubtimeout")
        self.assertTrue(t.available())
        t.apply()
        self.assertIn("GRUB_TIMEOUT=2", self.get("/etc/default/grub"))
        t.undo()
        self.assertEqual(self.get("/etc/default/grub"), "GRUB_DEFAULT=saved\nGRUB_TIMEOUT=5\n")

    def test_missing_target_file_not_available(self):
        self.assertFalse(ox.tweak_by_id("perf-grubtimeout").available())


class TweakUnitsAndPackages(Base):
    def test_enable_unit_and_restore(self):
        t = ox.tweak_by_id("perf-trim")
        self.assertTrue(t.apply())
        self.assertIn("fstrim.timer", self.sys.enabled)
        self.assertTrue(t.undo())
        self.assertNotIn("fstrim.timer", self.sys.enabled)

    def test_unit_already_enabled_stays_enabled_on_undo(self):
        self.sys.enabled.add("fstrim.timer")
        t = ox.tweak_by_id("perf-trim")
        t.apply()
        t.undo()
        self.assertIn("fstrim.timer", self.sys.enabled)

    def test_disable_unit_and_restore(self):
        self.put("/usr/lib/systemd/system/avahi-daemon.service", "[Unit]\n")
        self.sys.enabled |= {"avahi-daemon.service", "avahi-daemon.socket"}
        self.sys.active.add("avahi-daemon.service")
        t = ox.tweak_by_id("priv-avahi")
        t.apply()
        self.assertNotIn("avahi-daemon.service", self.sys.enabled)
        t.undo()
        self.assertIn("avahi-daemon.service", self.sys.enabled)

    def test_package_installed_and_kept(self):
        t = ox.tweak_by_id("perf-zram")
        self.assertTrue(t.apply())
        self.assertIn("zram-generator", self.sys.pkgs)
        self.assertTrue(self.sys.ran("systemctl", "start", "systemd-zram-setup@zram0.service"))
        t.undo()
        self.assertIn("zram-generator", self.sys.pkgs)   # kept, with a log message
        self.assertIsNone(self.get("/etc/systemd/zram-generator.conf"))

    def test_package_failure_aborts(self):
        self.sys.fail_pkgs.add("earlyoom")
        t = ox.tweak_by_id("perf-earlyoom")
        self.assertFalse(t.apply())
        self.assertNotIn("earlyoom.service", self.sys.enabled)


class Catalog(unittest.TestCase):
    def test_unique_ids_and_sane_content(self):
        ids = [t.id for t in ox.TWEAKS]
        self.assertEqual(len(ids), len(set(ids)))
        for t in ox.TWEAKS:
            self.assertTrue(t.name and t.desc and t.cat, t.id)
            self.assertIn(t.risk, ("safe", "moderate", "advanced"))
            for path in t.files:
                self.assertTrue(path.startswith("/etc/"), (t.id, path))
            for cmd in t.post + t.cmds + t.undo_post:
                self.assertIsInstance(cmd, list, t.id)
            self.assertFalse(t.tags - {"safe", "perf", "privacy", "max"}, t.id)

    def test_apps_catalog(self):
        self.assertGreater(len(ox.APPS), 90)
        pk = [a["pkg"] for a in ox.APPS]
        self.assertEqual(len(pk), len(set(pk)))
        self.assertTrue(all(a["src"] in "PAF" for a in ox.APPS))

    def test_version_matches_repo(self):
        v = open(os.path.join(os.path.dirname(__file__), "..", "..", "VERSION"), encoding="utf-8").read().strip()
        self.assertEqual(ox.__version__, v)


class Apps(Base):
    def test_install_official_verified(self):
        app = next(a for a in ox.APPS if a["pkg"] == "firefox")
        self.assertTrue(ox.install_app(app))
        self.assertIn("firefox", self.sys.pkgs)
        self.assertTrue(ox.install_app(app))        # already installed
        self.assertTrue(ox.remove_app(app))
        self.assertNotIn("firefox", self.sys.pkgs)

    def test_failed_install_reported(self):
        self.sys.fail_pkgs.add("firefox")
        self.assertFalse(ox.install_app(next(a for a in ox.APPS if a["pkg"] == "firefox")))

    def test_aur_runs_as_user_and_needs_helper(self):
        app = next(a for a in ox.APPS if a["pkg"] == "brave-bin")
        self.assertTrue(ox.install_app(app))
        cmd, user, _w = next(c for c in self.sys.commands if c[0][0] == "yay")
        self.assertEqual(user, "tester")
        self.sys.pkgs.clear()
        self.sys.have.discard("yay")
        self.assertFalse(ox.install_app(app))

    def test_flatpak(self):
        app = next(a for a in ox.APPS if a["src"] == "F")
        self.assertTrue(ox.install_app(app))
        self.sys.have.discard("flatpak")
        self.sys.pkgs.clear()
        self.assertFalse(ox.install_app(app))

    def test_parse_installed_size(self):
        self.assertEqual(ox.parse_installed_size("Installed Size  : 1.50 MiB\n"), int(1.5 * 1024 * 1024))
        self.assertEqual(ox.parse_installed_size("nothing"), 0)


class Cleaner(Base):
    def test_clean_home_cache(self):
        cache = ox.P(os.path.join(ox.CTX.home, ".cache", "pip"))
        os.makedirs(cache)
        with open(os.path.join(cache, "a.bin"), "wb") as fh:
            fh.write(b"x" * 5000)
        t = next(t for t in ox.CLEAN_TARGETS if t["id"] == "pip")
        self.assertEqual(ox.target_size(t), 5000)
        self.assertEqual(ox.clean_target(t), 5000)
        self.assertEqual(ox.target_size(t), 0)
        self.assertTrue(os.path.isdir(cache))

    def test_dry_run_keeps_files(self):
        cache = ox.P(os.path.join(ox.CTX.home, ".cache", "pip"))
        os.makedirs(cache)
        with open(os.path.join(cache, "a.bin"), "wb") as fh:
            fh.write(b"x")
        ox.CTX.dry_run = True
        ox.clean_target(next(t for t in ox.CLEAN_TARGETS if t["id"] == "pip"))
        self.assertTrue(os.path.exists(os.path.join(cache, "a.bin")))

    def test_pacman_cache_uses_paccache(self):
        ox.clean_target(next(t for t in ox.CLEAN_TARGETS if t["id"] == "pacman-cache"))
        self.assertTrue(self.sys.ran("paccache", "-rk2"))


class Startup(Base):
    DESKTOP = "[Desktop Entry]\nType=Application\nName=Updater\nExec=/usr/bin/updater\n"

    def test_disable_enable_system_entry(self):
        self.put("/etc/xdg/autostart/updater.desktop", self.DESKTOP)
        items = ox.autostart_items()
        self.assertEqual([i["name"] for i in items], ["Updater"])
        self.assertTrue(items[0]["enabled"])
        ox.set_autostart(items[0], False)
        self.assertFalse(ox.autostart_items()[0]["enabled"])
        self.assertIn("Hidden=true", self.get("/home/tester/.config/autostart/updater.desktop"))
        self.assertEqual(self.get("/etc/xdg/autostart/updater.desktop"), self.DESKTOP)   # system file untouched
        ox.set_autostart(ox.autostart_items()[0], True)
        self.assertTrue(ox.autostart_items()[0]["enabled"])


class Network(Base):
    def test_dns_set_and_restore(self):
        self.sys.nm[("Wired connection 1", "ipv4.dns")] = "192.168.1.1"
        self.assertTrue(ox.set_dns("cloudflare"))
        mod = next(c for c, _u, _w in self.sys.commands if c[:3] == ["nmcli", "connection", "modify"])
        self.assertIn("1.1.1.1,1.0.0.1", mod)
        self.assertEqual([c for c in ox.active_connections()][0]["name"], "Wired connection 1")
        self.assertTrue(ox.restore_dns())
        restore = [c for c, _u, _w in self.sys.commands if c[:3] == ["nmcli", "connection", "modify"]][-1]
        self.assertIn("192.168.1.1", restore)
        self.assertEqual([k for k in ox.load_state() if k.startswith("dns:")], [])


class Services(Base):
    def test_set_and_restore(self):
        self.sys.enabled.add("cups.service")
        self.assertTrue(ox.set_service("cups.service", False))
        self.assertNotIn("cups.service", self.sys.enabled)
        self.assertTrue(ox.restore_service("cups.service"))
        self.assertIn("cups.service", self.sys.enabled)
        rows = {r["unit"] for r in ox.service_rows()}
        self.assertIn("cups.service", rows)


class Cli(Base):
    def test_preset_apply_and_list(self):
        self.put("/etc/pacman.conf", "[options]\n#Color\n")
        self.assertEqual(ox.main(["tweaks", "apply", "--preset", "safe"]), 0)
        applied = [t.id for t in ox.TWEAKS if t.available() and t.is_applied()]
        self.assertIn("perf-vm", applied)
        self.assertNotIn("sec-ufw", applied)          # moderate: never in the safe preset
        self.assertEqual(ox.main(["tweaks", "undo", "--all"]), 0)
        self.assertEqual(ox.load_state(), {})

    def test_unknown_tweak_exit_code(self):
        self.assertEqual(ox.main(["tweaks", "apply", "nope"]), 1)

    def test_nothing_to_do(self):
        self.assertEqual(ox.main(["tweaks", "apply"]), 2)

    def test_refuses_non_arch(self):
        ox.CTX.runner = None
        self.put("/etc/os-release", "ID=ubuntu\n")
        self.assertEqual(ox.main(["info"]), 2)

    def test_version_helpers(self):
        self.assertEqual(ox.vtuple("v2.10.1"), (2, 10, 1))
        self.assertGreater(ox.vtuple("v2.10.0"), ox.vtuple("2.9.9"))


class Interactive(Base):
    def feed(self, answers):
        it = iter(answers)
        ox.ask = lambda prompt: next(it, "b")

    def test_menu_tweaks_preset_apply(self):
        self.put("/etc/pacman.conf", "[options]\n#Color\n")
        orig = ox.ask
        try:
            self.feed(["2", "p safe", "apply", "b", "q"])      # Tweaks -> preset -> apply -> back -> quit
            ox.interactive()
        finally:
            ox.ask = orig
        self.assertTrue(ox.tweak_by_id("perf-vm").is_applied())
        self.assertFalse(ox.tweak_by_id("sec-ufw").is_applied())

    def test_menu_toggle_numbers_and_filter(self):
        orig = ox.ask
        try:
            self.feed(["1", "/firefox", "1", "install", "b", "q"])
            ox.interactive()
        finally:
            ox.ask = orig
        self.assertIn("firefox", self.sys.pkgs)

    def test_help_parses(self):
        with self.assertRaises(SystemExit) as cm:
            ox.main(["--help"])
        self.assertEqual(cm.exception.code, 0)


if __name__ == "__main__":
    unittest.main()
