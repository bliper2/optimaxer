//! Disk cleaner. Cache folders in the user's home are emptied directly (nothing outside the target is touched, and a
//! redirected folder is refused); system-wide cleanups run as a previewed plan with administrator rights.
//! Deleted cache files cannot be restored, which the UI says before it asks.

use super::system::{Plan, Step, System};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Risk {
    Safe,
    Moderate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Folders below the user's home; their contents are deleted.
    Home(&'static [&'static str]),
    /// Journal vacuum, package manager cache, core dumps or unused Flatpak runtimes.
    Journal,
    CoreDumps,
    PackageCache,
    FlatpakUnused,
}

#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub id: &'static str,
    pub name: &'static str,
    pub risk: Risk,
    pub kind: Kind,
}

impl Target {
    pub fn needs_root(&self) -> bool {
        matches!(
            self.kind,
            Kind::Journal | Kind::CoreDumps | Kind::PackageCache
        )
    }
}

pub const TARGETS: [Target; 13] = [
    Target {
        id: "thumbs",
        name: "Thumbnail cache",
        risk: Risk::Safe,
        kind: Kind::Home(&[".cache/thumbnails"]),
    },
    Target {
        id: "firefox",
        name: "Firefox cache",
        risk: Risk::Safe,
        kind: Kind::Home(&[".cache/mozilla/firefox"]),
    },
    Target {
        id: "chromium",
        name: "Chromium / Chrome / Brave / Vivaldi cache",
        risk: Risk::Safe,
        kind: Kind::Home(&[
            ".cache/chromium",
            ".cache/google-chrome",
            ".cache/BraveSoftware",
            ".cache/vivaldi",
        ]),
    },
    Target {
        id: "yay",
        name: "AUR helper build cache (yay/paru)",
        risk: Risk::Safe,
        kind: Kind::Home(&[".cache/yay", ".cache/paru"]),
    },
    Target {
        id: "pip",
        name: "pip cache",
        risk: Risk::Safe,
        kind: Kind::Home(&[".cache/pip"]),
    },
    Target {
        id: "npm",
        name: "npm cache",
        risk: Risk::Safe,
        kind: Kind::Home(&[".npm/_cacache"]),
    },
    Target {
        id: "go",
        name: "Go build cache",
        risk: Risk::Safe,
        kind: Kind::Home(&[".cache/go-build"]),
    },
    Target {
        id: "cargo",
        name: "Rust cargo download cache",
        risk: Risk::Moderate,
        kind: Kind::Home(&[".cargo/registry/cache"]),
    },
    Target {
        id: "trash",
        name: "Trash",
        risk: Risk::Moderate,
        kind: Kind::Home(&[".local/share/Trash/files", ".local/share/Trash/info"]),
    },
    Target {
        id: "flatpak-unused",
        name: "Unused Flatpak runtimes (user installs)",
        risk: Risk::Safe,
        kind: Kind::FlatpakUnused,
    },
    Target {
        id: "pkg-cache",
        name: "Package manager download cache",
        risk: Risk::Safe,
        kind: Kind::PackageCache,
    },
    Target {
        id: "journal",
        name: "systemd journal (keep 100 MB)",
        risk: Risk::Safe,
        kind: Kind::Journal,
    },
    Target {
        id: "coredumps",
        name: "Core dumps",
        risk: Risk::Safe,
        kind: Kind::CoreDumps,
    },
];

pub fn home_paths(target: &Target, home: &Path) -> Vec<PathBuf> {
    match target.kind {
        Kind::Home(rel) => rel.iter().map(|r| home.join(r)).collect(),
        _ => Vec::new(),
    }
}

/// Bytes in regular files below `path`, without following links. Capped so a pathological tree cannot hang the scan.
pub fn dir_size(path: &Path) -> u64 {
    const MAX_ENTRIES: usize = 2_000_000;
    let mut total = 0u64;
    let mut seen = 0usize;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            seen += 1;
            if seen > MAX_ENTRIES {
                return total;
            }
            let Ok(meta) = fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                total += meta.len();
            }
        }
    }
    total
}

/// True when `base/rel` or any folder on the way below `base` is a symbolic link: a redirected cache folder must never
/// turn a cleanup into deleting somewhere else. Links above `base` (for example `/home` on atomic distros) are fine.
pub fn redirected_below(base: &Path, rel: &Path) -> bool {
    let mut current = base.to_path_buf();
    for part in rel.components() {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => return true,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
            Err(_) => return true,
        }
    }
    false
}

pub struct Scan {
    pub id: &'static str,
    /// `None` when the size cannot be read without administrator rights.
    pub bytes: Option<u64>,
    pub present: bool,
}

pub fn parse_journal_usage(text: &str) -> Option<u64> {
    let after = text
        .split("take up")
        .nth(1)
        .or_else(|| text.split("takes up").nth(1))?;
    let token = after.split_whitespace().next()?;
    let (digits, unit) = token.split_at(token.find(|c: char| c.is_ascii_alphabetic())?);
    let value: f64 = digits.parse().ok()?;
    let factor = match unit.chars().next()? {
        'B' => 1.0,
        'K' => 1024.0,
        'M' => 1024.0 * 1024.0,
        'G' => 1024.0 * 1024.0 * 1024.0,
        'T' => 1024.0_f64.powi(4),
        _ => return None,
    };
    Some((value * factor) as u64)
}

pub fn package_manager(sys: &dyn System) -> Option<&'static str> {
    ["pacman", "dnf", "apt-get"]
        .into_iter()
        .find(|pm| sys.have(pm))
}

fn pkg_cache_dirs(pm: &str) -> &'static [&'static str] {
    match pm {
        "pacman" => &["/var/cache/pacman/pkg"],
        "dnf" => &["/var/cache/libdnf5", "/var/cache/dnf"],
        _ => &["/var/cache/apt/archives"],
    }
}

pub fn scan(sys: &dyn System, home: &Path) -> Vec<Scan> {
    let pm = package_manager(sys);
    TARGETS
        .iter()
        .filter_map(|t| match t.kind {
            Kind::Home(_) => {
                let paths = home_paths(t, home);
                let present = paths.iter().any(|p| p.is_dir());
                Some(Scan {
                    id: t.id,
                    bytes: Some(paths.iter().map(|p| dir_size(p)).sum()),
                    present,
                })
            }
            Kind::FlatpakUnused => sys.have("flatpak").then_some(Scan {
                id: t.id,
                bytes: None,
                present: true,
            }),
            Kind::PackageCache => pm.map(|pm| {
                let paths = pkg_cache_dirs(pm);
                Scan {
                    id: t.id,
                    bytes: Some(paths.iter().map(|p| dir_size(Path::new(p))).sum()),
                    present: paths.iter().any(|p| Path::new(p).is_dir()),
                }
            }),
            Kind::Journal => {
                let usage = sys.run(&["journalctl", "--disk-usage"]);
                usage.ok().then(|| Scan {
                    id: t.id,
                    bytes: parse_journal_usage(&usage.text),
                    present: true,
                })
            }
            Kind::CoreDumps => {
                let path = Path::new("/var/lib/systemd/coredump");
                path.is_dir().then(|| Scan {
                    id: t.id,
                    bytes: Some(dir_size(path)),
                    present: true,
                })
            }
        })
        .collect()
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CleanResult {
    pub removed_entries: u64,
    pub skipped: Vec<String>,
}

/// Deletes everything inside each folder of a home target and keeps the folder itself.
pub fn clean_home_target(target: &Target, home: &Path) -> CleanResult {
    let mut result = CleanResult::default();
    let Kind::Home(rels) = target.kind else {
        return result;
    };
    for rel in rels {
        let dir = home.join(rel);
        if redirected_below(home, Path::new(rel)) {
            result.skipped.push(format!(
                "{} is a link to somewhere else, so it was left alone",
                dir.display()
            ));
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let removed = match fs::symlink_metadata(&path) {
                Ok(meta) if meta.is_dir() => fs::remove_dir_all(&path),
                Ok(_) => fs::remove_file(&path),
                Err(e) => Err(e),
            };
            match removed {
                Ok(()) => result.removed_entries += 1,
                Err(e) => result.skipped.push(format!("{}: {e}", path.display())),
            }
        }
    }
    result
}

/// Commands for a target that needs administrator rights, or `None` when nothing can be done on this system.
pub fn root_plan(sys: &dyn System, target: &Target) -> Option<Plan> {
    let mut plan = Plan::default();
    match target.kind {
        Kind::Journal => plan.push(Step::run(&["journalctl", "--vacuum-size=100M"])),
        Kind::CoreDumps => plan.push(Step::run(&[
            "find",
            "/var/lib/systemd/coredump",
            "-mindepth",
            "1",
            "-delete",
        ])),
        Kind::PackageCache => match package_manager(sys)? {
            "pacman" if sys.have("paccache") => {
                plan.push(Step::run(&["paccache", "-rk2"]));
                plan.push(Step::run(&["paccache", "-ruk0"]));
            }
            "pacman" => plan.push(Step::run(&["pacman", "-Sc", "--noconfirm"])),
            "dnf" => plan.push(Step::run(&["dnf", "clean", "all"])),
            _ => plan.push(Step::run(&["apt-get", "clean"])),
        },
        _ => return None,
    }
    Some(plan)
}

pub fn flatpak_plan() -> Plan {
    let mut plan = Plan::default();
    plan.push(Step::run_user(&[
        "flatpak",
        "uninstall",
        "--unused",
        "--user",
        "-y",
    ]));
    plan
}

#[cfg(test)]
mod tests {
    use super::super::system::fake::FakeSystem;
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "optimaxer-linux-clean-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn target(id: &str) -> Target {
        *TARGETS.iter().find(|t| t.id == id).unwrap()
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<_> = TARGETS.iter().map(|t| t.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), TARGETS.len());
    }

    #[test]
    fn sizes_count_files_in_nested_folders() {
        let home = temp("size");
        fs::create_dir_all(home.join(".cache/pip/a/b")).unwrap();
        fs::write(home.join(".cache/pip/a/b/f"), vec![0u8; 1000]).unwrap();
        fs::write(home.join(".cache/pip/g"), vec![0u8; 24]).unwrap();
        assert_eq!(dir_size(&home.join(".cache/pip")), 1024);
        assert_eq!(dir_size(&home.join("missing")), 0);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn cleaning_empties_the_folder_but_keeps_it_and_touches_nothing_else() {
        let home = temp("clean");
        let pip = home.join(".cache/pip");
        fs::create_dir_all(pip.join("wheels/x")).unwrap();
        fs::write(pip.join("wheels/x/w.whl"), "x").unwrap();
        fs::write(pip.join("http"), "y").unwrap();
        fs::write(home.join(".cache/keep.txt"), "keep").unwrap();
        let result = clean_home_target(&target("pip"), &home);
        assert_eq!(result.removed_entries, 2);
        assert!(result.skipped.is_empty());
        assert!(pip.is_dir() && fs::read_dir(&pip).unwrap().next().is_none());
        assert!(home.join(".cache/keep.txt").exists());
        fs::remove_dir_all(home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_redirected_cache_folder_is_refused_and_its_destination_survives() {
        let home = temp("redirect");
        let elsewhere = temp("redirect-target");
        fs::write(elsewhere.join("precious"), "data").unwrap();
        fs::create_dir_all(home.join(".cache")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, home.join(".cache/pip")).unwrap();
        let result = clean_home_target(&target("pip"), &home);
        assert_eq!(result.removed_entries, 0);
        assert_eq!(result.skipped.len(), 1);
        assert!(elsewhere.join("precious").exists());
        assert!(redirected_below(&home, Path::new(".cache/pip")));
        assert!(!redirected_below(&home, Path::new(".cache/other")));
        fs::remove_dir_all(home).unwrap();
        fs::remove_dir_all(elsewhere).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn links_inside_a_cache_are_removed_without_following_them() {
        let home = temp("inner");
        let elsewhere = temp("inner-target");
        fs::write(elsewhere.join("precious"), "data").unwrap();
        fs::create_dir_all(home.join(".cache/pip")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, home.join(".cache/pip/link")).unwrap();
        let result = clean_home_target(&target("pip"), &home);
        assert_eq!(result.removed_entries, 1);
        assert!(
            elsewhere.join("precious").exists(),
            "the link went away, not its destination"
        );
        fs::remove_dir_all(home).unwrap();
        fs::remove_dir_all(elsewhere).unwrap();
    }

    #[test]
    fn journal_usage_text_parses_in_every_unit() {
        assert_eq!(
            parse_journal_usage("Archived and active journals take up 1.5G in the file system.\n"),
            Some(1536 * 1024 * 1024)
        );
        assert_eq!(
            parse_journal_usage("Journals take up 96.0M in the file system."),
            Some(96 * 1024 * 1024)
        );
        assert_eq!(parse_journal_usage("nothing here"), None);
    }

    #[test]
    fn system_cleanups_are_plans_chosen_by_package_manager() {
        let pacman = FakeSystem::default()
            .with_tool("pacman")
            .with_tool("paccache");
        let plan = root_plan(&pacman, &target("pkg-cache")).unwrap();
        assert_eq!(
            plan.describe(),
            vec!["Run paccache -rk2", "Run paccache -ruk0"]
        );
        assert_eq!(
            root_plan(
                &FakeSystem::default().with_tool("pacman"),
                &target("pkg-cache")
            )
            .unwrap()
            .describe(),
            vec!["Run pacman -Sc --noconfirm"]
        );
        assert_eq!(
            root_plan(
                &FakeSystem::default().with_tool("dnf"),
                &target("pkg-cache")
            )
            .unwrap()
            .describe(),
            vec!["Run dnf clean all"]
        );
        assert!(root_plan(&FakeSystem::default(), &target("pkg-cache")).is_none());
        assert!(root_plan(&pacman, &target("journal")).unwrap().needs_root());
        assert!(!flatpak_plan().needs_root());
        assert!(root_plan(&pacman, &target("pip")).is_none());
    }

    #[test]
    fn scan_reports_home_targets_and_skips_unavailable_system_ones() {
        let home = temp("scan");
        fs::create_dir_all(home.join(".cache/pip")).unwrap();
        fs::write(home.join(".cache/pip/f"), vec![0u8; 10]).unwrap();
        let scans = scan(&FakeSystem::default(), &home);
        let pip = scans.iter().find(|s| s.id == "pip").unwrap();
        assert_eq!((pip.bytes, pip.present), (Some(10), true));
        assert!(!scans.iter().find(|s| s.id == "npm").unwrap().present);
        assert!(
            scans
                .iter()
                .all(|s| s.id != "journal" && s.id != "pkg-cache" && s.id != "flatpak-unused")
        );
        fs::remove_dir_all(home).unwrap();
    }
}
