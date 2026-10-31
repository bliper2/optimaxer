//! Tests run against a simulated system: commands go to FakeSys and files to a temp root.

use crate::catalog::*;
use crate::env::{Env, Pm, STATE_FILE};
use crate::ops::*;
use crate::tweaks::{LineEdit, Tweak};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Fake {
    pkgs: BTreeSet<String>,
    enabled: BTreeSet<String>,
    active: BTreeSet<String>,
    units: BTreeSet<String>,
    commands: Vec<(Vec<String>, bool, bool)>,
    have: BTreeSet<String>,
    fail_prefixes: Vec<Vec<String>>,
    fail_pkgs: BTreeSet<String>,
    nm: Vec<(String, String, String)>, // name, type, device
    nm_props: std::collections::BTreeMap<(String, String), String>,
    fail_nm_read: bool,
}

struct Sys {
    env: Env,
    fake: Arc<Mutex<Fake>>,
    _dir: tempfile::TempDir,
}

fn sys(pm: Pm) -> Sys {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Mutex::new(Fake::default()));
    {
        let mut f = fake.lock().unwrap();
        for h in ["pacman", "systemctl", "yay", "flatpak", "nmcli", "paccache", "sudo", "dnf", "rpm"] {
            f.have.insert(h.into());
        }
        for u in ["fstrim.timer", "earlyoom.service", "paccache.timer", "avahi-daemon.service", "NetworkManager-wait-online.service", "cups.service", "ufw.service"] {
            f.units.insert(u.into());
        }
    }
    let f2 = fake.clone();
    let mut env = Env::new(pm);
    env.root = dir.path().to_string_lossy().into_owned();
    env.user = Some("tester".into());
    env.home = Some("/home/tester".into());
    env.quiet = true;
    env.runner = Some(Arc::new(move |cmd, user, write, _input| {
        let mut f = f2.lock().unwrap();
        f.commands.push((cmd.to_vec(), user.is_some(), write));
        let c: Vec<&str> = cmd.iter().map(String::as_str).collect();
        if f.fail_prefixes.iter().any(|p| c.len() >= p.len() && c.iter().zip(p).all(|(a, b)| a == b)) {
            return (1, "simulated command failure".into());
        }
        let names = |from: usize| c[from..].iter().filter(|a| !a.starts_with('-')).map(|s| s.to_string()).collect::<Vec<_>>();
        match c.as_slice() {
            ["which", x] => (i32::from(!f.have.contains(*x)), String::new()),
            ["pacman", "-Q", p] | ["rpm", "-q", p] => (i32::from(!f.pkgs.contains(*p)), String::new()),
            ["pacman", "-S", ..] | ["dnf", "install", ..] | ["yay", ..] => {
                let n = names(if c[0] == "dnf" { 2 } else { 1 });
                if n.iter().any(|x| f.fail_pkgs.contains(x)) {
                    return (1, "error: target not found".into());
                }
                f.pkgs.extend(n);
                (0, String::new())
            }
            ["flatpak", "info", ..] => (1, String::new()),
            ["pacman", "-Rns", ..] => {
                for n in names(1) {
                    f.pkgs.remove(&n);
                }
                (0, String::new())
            }
            ["systemctl", "is-enabled", u] => if f.enabled.contains(*u) { (0, "enabled\n".into()) } else { (1, "disabled\n".into()) },
            ["systemctl", "is-active", u] => if f.active.contains(*u) { (0, "active\n".into()) } else { (3, "inactive\n".into()) },
            ["systemctl", "list-unit-files", u, ..] => (0, if f.units.contains(*u) { format!("{u} enabled\n") } else { String::new() }),
            ["systemctl", "enable", rest @ ..] => {
                let u = rest.last().unwrap().to_string();
                f.enabled.insert(u.clone());
                if rest.contains(&"--now") {
                    f.active.insert(u);
                }
                (0, String::new())
            }
            ["systemctl", "disable", rest @ ..] => {
                let u = rest.last().unwrap().to_string();
                f.enabled.remove(&u);
                if rest.contains(&"--now") {
                    f.active.remove(&u);
                }
                (0, String::new())
            }
            ["systemctl", "start", u] => {
                f.active.insert(u.to_string());
                (0, String::new())
            }
            ["systemctl", "stop", u] => {
                f.active.remove(*u);
                (0, String::new())
            }
            ["nmcli", "-t", ..] => (0, f.nm.iter().map(|(n, t, d)| format!("{}:{t}:{d}\n", n.replace(':', "\\:"))).collect()),
            ["nmcli", "-g", prop, "connection", "show", name] => {
                if f.fail_nm_read {
                    return (1, "boom".into());
                }
                (0, format!("{}\n", f.nm_props.get(&(name.to_string(), prop.to_string())).cloned().unwrap_or_default()))
            }
            ["nmcli", "connection", "modify", name, rest @ ..] => {
                for kv in rest.chunks(2) {
                    f.nm_props.insert((name.to_string(), kv[0].to_string()), kv[1].to_string());
                }
                (0, String::new())
            }
            _ => (0, String::new()),
        }
    }));
    Sys { env, fake, _dir: dir }
}

impl Sys {
    fn write(&self, path: &str, text: &str) {
        let full = self.env.p(path);
        std::fs::create_dir_all(std::path::Path::new(&full).parent().unwrap()).unwrap();
        std::fs::write(full, text).unwrap();
    }
    fn read(&self, path: &str) -> Option<String> {
        self.env.read_text(path)
    }
    fn ran(&self, prefix: &[&str]) -> bool {
        self.fake.lock().unwrap().commands.iter().any(|(c, _, _)| c.len() >= prefix.len() && c.iter().zip(prefix).all(|(a, b)| a == b))
    }
    fn fail(&self, prefix: &[&str]) {
        self.fake.lock().unwrap().fail_prefixes.push(prefix.iter().map(|s| s.to_string()).collect());
    }
}

fn simple_file_tweak() -> Tweak {
    Tweak::new("t-file", "File tweak", "d", "Test").file("/etc/test.conf", "a=1\n").post(&[&["true"]])
}

// ------------------------------------------------------------------------------------------------ line edits

#[test]
fn line_edit_uncomments_and_restores() {
    let e = LineEdit::new("/x", "ParallelDownloads", Some("5"), Some("options"), " = ");
    let (new, orig) = e.apply("[options]\n#ParallelDownloads = 3\nColor\n");
    assert_eq!(new, "[options]\nParallelDownloads = 5\nColor\n");
    assert_eq!(orig.as_deref(), Some("#ParallelDownloads = 3"));
    assert_eq!(e.revert(&new, orig.as_deref()), "[options]\n#ParallelDownloads = 3\nColor\n");
}

#[test]
fn line_edit_inserts_into_section_and_removes() {
    let e = LineEdit::new("/x", "Color", None, Some("options"), " = ");
    let (new, orig) = e.apply("[options]\nHoldPkg = pacman\n[core]\n");
    assert_eq!(new, "[options]\nColor\nHoldPkg = pacman\n[core]\n");
    assert!(orig.is_none());
    assert_eq!(e.revert(&new, None), "[options]\nHoldPkg = pacman\n[core]\n");
}

#[test]
fn line_edit_already_set_is_recognised_whatever_the_spacing() {
    let e = LineEdit::new("/x", "GRUB_TIMEOUT", Some("2"), None, "=");
    let (new, orig) = e.apply("GRUB_TIMEOUT = 2\n");
    assert_eq!(new, "GRUB_TIMEOUT = 2\n");
    assert!(orig.is_some());
}

#[test]
fn line_edit_appends_before_trailing_newline() {
    let e = LineEdit::new("/x", "K", Some("v"), None, "=");
    assert_eq!(e.apply("a=1\n").0, "a=1\nK=v\n");
    assert_eq!(e.apply("a=1").0, "a=1\nK=v");
}

#[test]
fn key_prefix_is_not_confused_with_longer_keys() {
    let e = LineEdit::new("/x", "Color", None, None, " = ");
    let (new, _) = e.apply("ColorTheme = dark\n");
    assert_eq!(new, "ColorTheme = dark\nColor\n");
}

// ------------------------------------------------------------------------------------------------ tweaks

#[test]
fn apply_and_undo_files() {
    let s = sys(Pm::Pacman);
    let t = simple_file_tweak();
    assert!(!t.is_applied(&s.env));
    assert!(t.apply(&s.env));
    assert!(t.is_applied(&s.env));
    assert!(t.undo(&s.env));
    assert_eq!(s.read("/etc/test.conf"), None);
    assert!(!s.read(STATE_FILE).unwrap().contains("t-file"));
}

#[test]
fn existing_file_is_restored() {
    let s = sys(Pm::Pacman);
    s.write("/etc/test.conf", "old\n");
    let t = simple_file_tweak();
    assert!(t.apply(&s.env));
    assert_eq!(s.read("/etc/test.conf").as_deref(), Some("a=1\n"));
    assert!(t.undo(&s.env));
    assert_eq!(s.read("/etc/test.conf").as_deref(), Some("old\n"));
}

#[test]
fn originals_survive_a_failed_command_and_retry() {
    let s = sys(Pm::Pacman);
    s.write("/etc/test.conf", "old\n");
    s.fail(&["true"]);
    let t = simple_file_tweak();
    assert!(!t.apply(&s.env), "a failed post command must not report success");
    assert!(s.read(STATE_FILE).unwrap().contains("old"));
    // retry after the file drifted: the first saved original must stay
    s.write("/etc/test.conf", "drifted\n");
    assert!(!t.apply(&s.env));
    s.fake.lock().unwrap().fail_prefixes.clear();
    assert!(t.apply(&s.env));
    assert!(t.undo(&s.env));
    assert_eq!(s.read("/etc/test.conf").as_deref(), Some("old\n"));
}

#[test]
fn failed_undo_command_keeps_originals_for_retry() {
    let s = sys(Pm::Pacman);
    let t = Tweak::new("t-undo", "n", "d", "c").file("/etc/u.conf", "x\n").undo_post(&[&["reload-thing"]]);
    assert!(t.apply(&s.env));
    s.fail(&["reload-thing"]);
    assert!(!t.undo(&s.env));
    assert!(s.read(STATE_FILE).unwrap().contains("t-undo"));
    s.fake.lock().unwrap().fail_prefixes.clear();
    assert!(t.undo(&s.env));
    assert!(!s.read(STATE_FILE).unwrap().contains("t-undo"));
}

#[test]
fn dry_run_changes_nothing() {
    let mut s = sys(Pm::Pacman);
    s.env.dry_run = true;
    let t = simple_file_tweak();
    t.apply(&s.env);
    assert_eq!(s.read("/etc/test.conf"), None);
    assert_eq!(s.read(STATE_FILE), None);
}

#[test]
fn unreadable_state_is_not_overwritten() {
    let s = sys(Pm::Pacman);
    s.write(STATE_FILE, "{ not json");
    assert!(!simple_file_tweak().apply(&s.env));
    assert_eq!(s.read(STATE_FILE).as_deref(), Some("{ not json"));
    assert_eq!(s.read("/etc/test.conf"), None);
}

#[test]
fn undo_of_unknown_tweak_fails() {
    let s = sys(Pm::Pacman);
    assert!(!simple_file_tweak().undo(&s.env));
}

#[test]
fn config_edit_is_backed_up_and_restored() {
    let s = sys(Pm::Pacman);
    s.write("/etc/pacman.conf", "[options]\n#ParallelDownloads = 3\n#Color\n");
    let t = tweaks().into_iter().find(|t| t.id == "pm-pacman").unwrap();
    assert!(t.available(&s.env));
    assert!(t.apply(&s.env));
    assert!(s.read("/etc/pacman.conf").unwrap().contains("ParallelDownloads = 5"));
    assert!(std::fs::read_dir(s.env.p("/var/lib/optimaxer/backups")).unwrap().count() == 1);
    assert!(t.undo(&s.env));
    assert_eq!(s.read("/etc/pacman.conf").as_deref(), Some("[options]\n#ParallelDownloads = 3\n#Color\n"));
}

#[test]
fn enable_unit_and_restore_keeps_prior_state() {
    let s = sys(Pm::Pacman);
    let t = Tweak::new("t-unit", "n", "d", "c").enable(&["fstrim.timer"]);
    assert!(t.apply(&s.env));
    assert!(s.fake.lock().unwrap().enabled.contains("fstrim.timer"));
    assert!(t.undo(&s.env));
    assert!(!s.fake.lock().unwrap().enabled.contains("fstrim.timer"));

    s.fake.lock().unwrap().enabled.insert("fstrim.timer".into());
    assert!(t.apply(&s.env));
    assert!(t.undo(&s.env));
    assert!(s.fake.lock().unwrap().enabled.contains("fstrim.timer"), "already enabled must stay enabled");
}

#[test]
fn disabled_but_active_unit_is_restored_exactly() {
    let s = sys(Pm::Pacman);
    s.fake.lock().unwrap().active.insert("cups.service".into());
    let t = Tweak::new("t-dis", "n", "d", "c").disable(&["cups.service"]);
    s.fake.lock().unwrap().enabled.insert("cups.service".into());
    assert!(t.apply(&s.env));
    assert!(t.undo(&s.env));
    let f = s.fake.lock().unwrap();
    assert!(f.enabled.contains("cups.service") && f.active.contains("cups.service"));
}

#[test]
fn package_installed_and_kept_and_failure_aborts() {
    let s = sys(Pm::Pacman);
    let t = Tweak::new("t-pkg", "n", "d", "c").pkgs(&["earlyoom"]).enable(&["earlyoom.service"]);
    assert!(t.apply(&s.env));
    assert!(s.fake.lock().unwrap().pkgs.contains("earlyoom"));
    assert!(t.undo(&s.env));
    assert!(s.fake.lock().unwrap().pkgs.contains("earlyoom"), "packages stay installed on undo");

    let s = sys(Pm::Pacman);
    s.fake.lock().unwrap().fail_pkgs.insert("earlyoom".into());
    assert!(!t.apply(&s.env));
    assert!(!s.ran(&["systemctl", "enable"]));
}

#[test]
fn tweaks_use_dnf_on_fedora() {
    let s = sys(Pm::Dnf);
    s.write("/etc/dnf/dnf.conf", "[main]\ngpgcheck=1\n");
    let all = tweaks();
    let pm_dnf = all.iter().find(|t| t.id == "pm-dnf").unwrap();
    let pm_pacman = all.iter().find(|t| t.id == "pm-pacman").unwrap();
    assert!(pm_dnf.available(&s.env) && !pm_pacman.available(&s.env));
    assert!(pm_dnf.apply(&s.env));
    assert!(s.read("/etc/dnf/dnf.conf").unwrap().contains("max_parallel_downloads=10"));
    let z = all.iter().find(|t| t.id == "perf-zram").unwrap();
    assert!(z.apply(&s.env));
    assert!(s.ran(&["dnf", "install", "-y", "zram-generator"]));
}

// ------------------------------------------------------------------------------------------------ catalog

#[test]
fn catalog_ids_are_unique_and_presets_exist() {
    let all = tweaks();
    let ids: BTreeSet<_> = all.iter().map(|t| t.id.clone()).collect();
    assert_eq!(ids.len(), all.len());
    assert_eq!(all.len(), 30);
    for t in &all {
        assert!(!t.name.is_empty() && !t.desc.is_empty());
        assert!(["safe", "moderate"].contains(&t.risk.as_str()), "{}", t.id);
        assert!(!(t.files.is_empty() && t.edits.is_empty() && t.pkgs.is_empty() && t.enable.is_empty() && t.disable.is_empty() && t.cmds.is_empty()), "{} does nothing", t.id);
        assert!(t.tags.iter().all(|x| PRESETS.contains(&x.as_str())), "{}", t.id);
    }
}

#[test]
fn browser_policies_are_valid_and_complete() {
    let all = tweaks();
    for id in ["browser-firefox", "browser-chromium", "browser-chrome", "browser-brave", "browser-edge"] {
        let t = all.iter().find(|t| t.id == id).unwrap();
        for (path, content) in &t.files {
            if path.ends_with(".json") {
                let v: serde_json::Value = serde_json::from_str(content).unwrap();
                assert!(v.is_object());
            }
        }
    }
    let ff = all.iter().find(|t| t.id == "browser-firefox").unwrap();
    let doc: serde_json::Value = serde_json::from_str(&ff.files[0].1).unwrap();
    assert_eq!(doc["policies"]["DisableTelemetry"], true);
    assert_eq!(doc["policies"]["OverrideFirstRunPage"], "");
    assert!(ff.files.iter().any(|(p, c)| p.ends_with("optimaxer.cfg") && c.contains("lockPref(\"browser.newtabpage.activity-stream.showWeather\", false);")));
    let brave: serde_json::Value = serde_json::from_str(&all.iter().find(|t| t.id == "browser-brave").unwrap().files[0].1).unwrap();
    assert_eq!(brave["BraveRewardsDisabled"], true);
    assert_eq!(brave["MetricsReportingEnabled"], false);
    // key order is stable so is_applied keeps matching what was written
    assert!(ff.files[0].1.starts_with("{\n  \"policies\": {\n    \"DisableTelemetry\""));
}

#[test]
fn browser_tweaks_need_the_browser() {
    let s = sys(Pm::Pacman);
    let t = tweaks().into_iter().find(|t| t.id == "browser-chromium").unwrap();
    assert!(!t.available(&s.env));
    s.fake.lock().unwrap().have.insert("chromium".into());
    assert!(t.available(&s.env));
}

#[test]
fn preset_selection_only_includes_available_tweaks() {
    let s = sys(Pm::Pacman);
    let ids = preset_ids(&s.env, "safe");
    assert!(ids.contains(&"perf-vm".to_string()));
    assert!(!ids.contains(&"browser-firefox".to_string()));
    assert!(!ids.contains(&"pm-dnf".to_string()));
}

#[test]
fn app_catalogs() {
    let arch = load_apps(Pm::Pacman);
    assert_eq!(arch.len(), 97);
    assert!(arch.iter().any(|a| a.src == 'A'));
    let fed = load_apps(Pm::Dnf);
    assert!(fed.iter().all(|a| a.src != 'A'));
    assert!(fed.iter().any(|a| a.pkg == "com.brave.Browser" && a.src == 'F'));
    assert!(fed.iter().any(|a| a.pkg == "ffmpeg-free"));
}

#[test]
fn install_official_app_is_verified_and_failure_reported() {
    let s = sys(Pm::Pacman);
    let app = App { cat: "c".into(), name: "Firefox".into(), src: 'P', pkg: "firefox".into() };
    assert!(install_app(&s.env, &app));
    assert!(app_installed(&s.env, &app));
    assert!(remove_app(&s.env, &app));
    assert!(!app_installed(&s.env, &app));
    s.fake.lock().unwrap().fail_pkgs.insert("firefox".into());
    assert!(!install_app(&s.env, &app));
}

#[test]
fn aur_app_runs_as_user_and_needs_a_helper() {
    let s = sys(Pm::Pacman);
    let app = App { cat: "c".into(), name: "Brave".into(), src: 'A', pkg: "brave-bin".into() };
    assert!(install_app(&s.env, &app));
    assert!(s.fake.lock().unwrap().commands.iter().any(|(c, user, _)| c[0] == "yay" && *user));
    let s = sys(Pm::Pacman);
    s.fake.lock().unwrap().have.remove("yay");
    assert!(!install_app(&s.env, &app));
}

#[test]
fn flatpak_adds_flathub_once_when_missing() {
    let s = sys(Pm::Dnf);
    let app = App { cat: "c".into(), name: "Discord".into(), src: 'F', pkg: "com.discordapp.Discord".into() };
    install_app(&s.env, &app);
    assert!(s.ran(&["flatpak", "remote-add"]));
    assert!(s.ran(&["flatpak", "install", "-y", "--noninteractive", "flathub", "com.discordapp.Discord"]));
}

#[test]
fn parse_installed_size_units() {
    use crate::sys::parse_installed_size;
    assert_eq!(parse_installed_size("Name : x\nInstalled Size  : 1.50 MiB\n"), 1_572_864);
    assert_eq!(parse_installed_size("Installed Size : 12,5 KiB"), 12_800);
    assert_eq!(parse_installed_size("nothing"), 0);
}

// ------------------------------------------------------------------------------------------------ cleaner

#[test]
fn clean_home_cache_and_dry_run() {
    let mut s = sys(Pm::Pacman);
    s.write("/home/tester/.cache/pip/a.whl", "12345");
    s.write("/home/tester/.cache/pip/sub/b.whl", "12345");
    let t = clean_targets(Pm::Pacman).into_iter().find(|t| t.id == "pip").unwrap();
    assert_eq!(target_size(&s.env, &t), 10);
    s.env.dry_run = true;
    assert_eq!(clean_target(&s.env, &t), 0, "dry run never claims freed bytes");
    assert!(s.read("/home/tester/.cache/pip/a.whl").is_some());
    s.env.dry_run = false;
    assert_eq!(clean_target(&s.env, &t), 10);
    assert!(s.read("/home/tester/.cache/pip/a.whl").is_none());
    assert!(std::path::Path::new(&s.env.p("/home/tester/.cache/pip")).is_dir(), "the cache folder itself stays");
}

#[test]
fn redirected_target_is_not_followed() {
    let s = sys(Pm::Pacman);
    s.write("/elsewhere/precious.txt", "keep");
    std::fs::create_dir_all(s.env.p("/home/tester/.cache")).unwrap();
    std::os::unix::fs::symlink(s.env.p("/elsewhere"), s.env.p("/home/tester/.cache/pip")).unwrap();
    let t = clean_targets(Pm::Pacman).into_iter().find(|t| t.id == "pip").unwrap();
    clean_target(&s.env, &t);
    assert_eq!(s.read("/elsewhere/precious.txt").as_deref(), Some("keep"));
}

#[test]
fn symlinks_inside_a_cache_are_removed_without_following() {
    let s = sys(Pm::Pacman);
    s.write("/elsewhere/precious.txt", "keep");
    std::fs::create_dir_all(s.env.p("/home/tester/.cache/pip")).unwrap();
    std::os::unix::fs::symlink(s.env.p("/elsewhere"), s.env.p("/home/tester/.cache/pip/link")).unwrap();
    let t = clean_targets(Pm::Pacman).into_iter().find(|t| t.id == "pip").unwrap();
    clean_target(&s.env, &t);
    assert_eq!(s.read("/elsewhere/precious.txt").as_deref(), Some("keep"));
    assert!(std::fs::symlink_metadata(s.env.p("/home/tester/.cache/pip/link")).is_err());
}

#[test]
fn a_symlinked_parent_outside_the_home_does_not_block_cleaning() {
    // /home -> /var/home style layouts: only links between the home folder and the target matter.
    let s = sys(Pm::Pacman);
    s.write("/real/tester/.cache/pip/a", "123");
    std::os::unix::fs::symlink(s.env.p("/real"), s.env.p("/home")).unwrap();
    let t = clean_targets(Pm::Pacman).into_iter().find(|t| t.id == "pip").unwrap();
    assert_eq!(clean_target(&s.env, &t), 3);
}

#[test]
fn cleaner_targets_follow_the_package_manager() {
    let arch: Vec<_> = clean_targets(Pm::Pacman).iter().map(|t| t.id).collect();
    let fed: Vec<_> = clean_targets(Pm::Dnf).iter().map(|t| t.id).collect();
    assert!(arch.contains(&"pacman-cache") && !arch.contains(&"dnf-cache"));
    assert!(fed.contains(&"dnf-cache") && !fed.contains(&"pacman-cache") && !fed.contains(&"yay"));
}

// ------------------------------------------------------------------------------------------------ startup

const ENTRY: &str = "[Desktop Entry]\nName=Thing\nExec=thing --go\nX-GNOME-Autostart-enabled=true\n";

#[test]
fn disable_and_enable_a_system_autostart_entry() {
    let s = sys(Pm::Pacman);
    s.write("/etc/xdg/autostart/thing.desktop", ENTRY);
    let item = autostart_items(&s.env).remove(0);
    assert!(item.enabled && item.scope == "system" && item.exec == "thing --go");
    assert!(set_autostart(&s.env, &item, false));
    let item = autostart_items(&s.env).remove(0);
    assert!(!item.enabled);
    assert_eq!(s.read("/etc/xdg/autostart/thing.desktop").as_deref(), Some(ENTRY), "system file is never edited");
    assert!(set_autostart(&s.env, &item, true));
    assert!(autostart_items(&s.env).remove(0).enabled);
}

#[test]
fn enable_clears_the_gnome_disabled_flag() {
    let s = sys(Pm::Pacman);
    s.write("/home/tester/.config/autostart/thing.desktop", "[Desktop Entry]\nName=Thing\nExec=thing\nX-GNOME-Autostart-enabled=false\n");
    let item = autostart_items(&s.env).remove(0);
    assert!(!item.enabled);
    assert!(set_autostart(&s.env, &item, true));
    assert!(autostart_items(&s.env).remove(0).enabled);
}

// ------------------------------------------------------------------------------------------------ services

#[test]
fn service_set_and_restore() {
    let s = sys(Pm::Pacman);
    s.fake.lock().unwrap().enabled.insert("cups.service".into());
    s.fake.lock().unwrap().active.insert("cups.service".into());
    assert!(service_rows(&s.env).iter().any(|r| r.unit == "cups.service" && r.enabled && r.active));
    assert!(set_service(&s.env, "cups.service", false));
    assert!(!s.fake.lock().unwrap().enabled.contains("cups.service"));
    assert!(restore_service(&s.env, "cups.service"));
    let f = s.fake.lock().unwrap();
    assert!(f.enabled.contains("cups.service") && f.active.contains("cups.service"));
    drop(f);
    assert!(!s.read(STATE_FILE).unwrap().contains("service:cups"));
}

#[test]
fn failed_service_restore_keeps_the_record() {
    let s = sys(Pm::Pacman);
    s.fake.lock().unwrap().enabled.insert("cups.service".into());
    assert!(set_service(&s.env, "cups.service", false));
    s.fail(&["systemctl", "enable"]);
    assert!(!restore_service(&s.env, "cups.service"));
    assert!(s.read(STATE_FILE).unwrap().contains("service:cups.service"));
}

#[test]
fn undo_all_restores_tweaks_services_and_dns() {
    let s = sys(Pm::Pacman);
    let vm = tweaks().into_iter().find(|t| t.id == "perf-vm").unwrap();
    assert!(vm.apply(&s.env));
    s.fake.lock().unwrap().enabled.insert("cups.service".into());
    assert!(set_service(&s.env, "cups.service", false));
    assert_eq!(undoable_count(&s.env), 2);
    assert!(undo_all(&s.env));
    assert_eq!(undoable_count(&s.env), 0);
    assert!(s.read("/etc/sysctl.d/99-optimaxer-vm.conf").is_none());
    assert!(s.fake.lock().unwrap().enabled.contains("cups.service"));
}

// ------------------------------------------------------------------------------------------------ network

#[test]
fn nmcli_escapes_are_decoded() {
    assert_eq!(split_nmcli(r"Home\: 5G:802-11-wireless:wlan0"), vec!["Home: 5G", "802-11-wireless", "wlan0"]);
    assert_eq!(split_nmcli(r"a\\b:ethernet:eth0"), vec![r"a\b", "ethernet", "eth0"]);
}

fn wifi(s: &Sys) {
    let mut f = s.fake.lock().unwrap();
    f.nm.push(("Home: 5G".into(), "802-11-wireless".into(), "wlan0".into()));
    f.nm.push(("docker0".into(), "bridge".into(), "docker0".into()));
    f.nm_props.insert(("Home: 5G".into(), "ipv4.dns".into()), "192.168.1.1".into());
    f.nm_props.insert(("Home: 5G".into(), "ipv4.ignore-auto-dns".into()), "no".into());
}

#[test]
fn dns_set_and_restore() {
    let s = sys(Pm::Pacman);
    wifi(&s);
    assert_eq!(active_connections(&s.env).len(), 1);
    assert!(set_dns(&s.env, "cloudflare"));
    assert_eq!(s.fake.lock().unwrap().nm_props[&("Home: 5G".to_string(), "ipv4.dns".to_string())], "1.1.1.1,1.0.0.1");
    assert!(s.ran(&["nmcli", "device", "reapply", "wlan0"]));
    assert!(restore_dns(&s.env));
    let f = s.fake.lock().unwrap();
    assert_eq!(f.nm_props[&("Home: 5G".to_string(), "ipv4.dns".to_string())], "192.168.1.1");
    assert_eq!(f.nm_props[&("Home: 5G".to_string(), "ipv4.ignore-auto-dns".to_string())], "no");
    drop(f);
    assert!(!s.read(STATE_FILE).unwrap().contains("dns:"));
}

#[test]
fn dns_failures_keep_state_honest() {
    let s = sys(Pm::Pacman);
    wifi(&s);
    s.fake.lock().unwrap().fail_nm_read = true;
    assert!(!set_dns(&s.env, "google"));
    assert!(!s.ran(&["nmcli", "connection", "modify"]), "a failed snapshot changes nothing");
    s.fake.lock().unwrap().fail_nm_read = false;
    s.fail(&["nmcli", "device", "reapply"]);
    assert!(!set_dns(&s.env, "google"), "a failed reapply is reported");
    s.fake.lock().unwrap().fail_prefixes.clear();
    s.fail(&["nmcli", "connection", "modify"]);
    assert!(!restore_dns(&s.env));
    assert!(s.read(STATE_FILE).unwrap().contains("dns:Home: 5G"), "failed restore keeps the record");
}

#[test]
fn dns_without_connections_or_unknown_provider_is_not_success() {
    let s = sys(Pm::Pacman);
    assert!(!set_dns(&s.env, "cloudflare"));
    wifi(&s);
    assert!(!set_dns(&s.env, "nope"));
    assert!(!restore_dns(&s.env));
}

// ------------------------------------------------------------------------------------------------ misc

#[test]
fn version_helpers() {
    assert!(vtuple("v2.15.41") > vtuple("2.15.40"));
    assert_eq!(vtuple("v2.15.40"), (2, 15, 40));
    assert_eq!(vtuple("garbage"), (0, 0, 0));
}

#[test]
fn cargo_version_matches_repo_version_file() {
    let v = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../VERSION")).unwrap();
    assert_eq!(v.trim(), crate::env::VERSION);
}

#[test]
fn distribution_detection() {
    let mut s = sys(Pm::Pacman);
    let cases = [("ID=manjaro\nID_LIKE=arch\n", Some(Pm::Pacman)), ("ID=endeavouros\nID_LIKE=arch\n", Some(Pm::Pacman)), ("ID=fedora\n", Some(Pm::Dnf)),
        ("ID=rocky\nID_LIKE=\"rhel centos fedora\"\n", Some(Pm::Dnf)), ("ID=ubuntu\nID_LIKE=debian\n", None)];
    for (text, want) in cases {
        s.write("/etc/os-release", text);
        assert_eq!(crate::env::detect_pm(&s.env), want, "{text}");
    }
    s.env.root.clear();
}

#[test]
fn config_round_trip_applies_tweaks_then_apps() {
    let s = sys(Pm::Pacman);
    let cfg = config_file(vec!["perf-vm".into(), "nope".into()], vec!["firefox".into()]);
    assert_eq!(cfg.tweaks, vec!["nope", "perf-vm"]);
    let json = serde_json::to_string(&cfg).unwrap();
    let back: ConfigFile = serde_json::from_str(&json).unwrap();
    assert!(!apply_config(&s.env, &back), "an unknown tweak makes the whole run report failure");
    assert!(s.read("/etc/sysctl.d/99-optimaxer-vm.conf").is_some());
    assert!(s.fake.lock().unwrap().pkgs.contains("firefox"));
}

#[test]
fn launcher_and_dry_run() {
    let mut s = sys(Pm::Pacman);
    s.env.dry_run = true;
    assert!(create_launcher(&s.env).unwrap().is_empty());
    assert!(install_command(&s.env).unwrap().is_empty());
    s.env.dry_run = false;
    let made = create_launcher(&s.env).unwrap();
    assert_eq!(made.len(), 1, "no Desktop folder, so only the menu entry");
    let entry = std::fs::read_to_string(&made[0]).unwrap();
    assert!(entry.contains("Exec=") && entry.contains(" gui\n"));
    assert!(std::path::Path::new(&s.env.p("/home/tester/.local/share/icons/hicolor/256x256/apps/optimaxer.png")).is_file());
}

#[test]
fn cli_definition_is_consistent() {
    use clap::CommandFactory;
    crate::Cli::command().debug_assert();
}

#[test]
fn menu_number_ranges_toggle() {
    let mut sel = BTreeSet::new();
    crate::menu::toggle_tokens("1 4 7-9, 99", 10, &mut sel);
    assert_eq!(sel.into_iter().collect::<Vec<_>>(), vec![0, 3, 6, 7, 8]);
    let mut sel: BTreeSet<usize> = [0].into();
    crate::menu::toggle_tokens("1", 10, &mut sel);
    assert!(sel.is_empty());
}
