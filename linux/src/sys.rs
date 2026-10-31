//! systemd, package-manager and Flatpak helpers.

use crate::env::{Env, Pm, last_line, s, tail};

pub fn unit_exists(env: &Env, unit: &str) -> bool {
    let (rc, out) = env.run(&["systemctl", "list-unit-files", unit, "--no-legend"]);
    rc == 0 && out.contains(unit.split('.').next().unwrap_or(unit))
}

pub fn unit_enabled(env: &Env, unit: &str) -> bool {
    let (_, out) = env.run(&["systemctl", "is-enabled", unit]);
    matches!(out.trim(), "enabled" | "enabled-runtime")
}

pub fn unit_active(env: &Env, unit: &str) -> bool {
    env.run(&["systemctl", "is-active", unit]).0 == 0
}

/// Enabled at boot and running now are independent states, including disabled but active units.
pub fn restore_unit(env: &Env, unit: &str, enabled: bool, active: bool) -> bool {
    let e = env.change_s(&["systemctl", if enabled { "enable" } else { "disable" }, unit]);
    let a = env.change_s(&["systemctl", if active { "start" } else { "stop" }, unit]);
    e && a && (env.dry_run || (unit_enabled(env, unit) == enabled && unit_active(env, unit) == active))
}

pub fn pkg_installed(env: &Env, pkg: &str) -> bool {
    match env.pm {
        Pm::Pacman => env.run(&["pacman", "-Q", pkg]).0 == 0,
        Pm::Dnf => env.run(&["rpm", "-q", pkg]).0 == 0,
    }
}

pub fn pkg_install(env: &Env, pkgs: &[String]) -> (i32, String) {
    let mut cmd = match env.pm {
        Pm::Pacman => s(&["pacman", "-S", "--needed", "--noconfirm"]),
        Pm::Dnf => s(&["dnf", "install", "-y"]),
    };
    cmd.extend(pkgs.iter().cloned());
    env.exec(&cmd, true, false, None)
}

pub fn pkg_remove(env: &Env, pkgs: &[String]) -> (i32, String) {
    let mut cmd = match env.pm {
        Pm::Pacman => s(&["pacman", "-Rns", "--noconfirm"]),
        Pm::Dnf => s(&["dnf", "remove", "-y"]),
    };
    cmd.extend(pkgs.iter().cloned());
    env.exec(&cmd, true, false, None)
}

/// Packages installed as dependencies that nothing needs any more.
pub fn orphans(env: &Env) -> Vec<String> {
    match env.pm {
        Pm::Pacman => {
            let (rc, out) = env.run(&["pacman", "-Qtdq"]);
            if rc == 0 { out.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect() } else { vec![] }
        }
        Pm::Dnf => {
            let (rc, out) = env.run(&["dnf", "repoquery", "--unneeded", "--qf", "%{name}"]);
            if rc != 0 {
                return vec![];
            }
            let mut seen: Vec<String> = vec![];
            for l in out.lines().map(str::trim) {
                if !l.is_empty() && !["Last metadata", "Updating", "Repositories"].iter().any(|p| l.starts_with(p)) && !seen.iter().any(|x| x == l) {
                    seen.push(l.to_string());
                }
            }
            seen
        }
    }
}

pub fn parse_installed_size(info: &str) -> u64 {
    for line in info.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        if k.trim() != "Installed Size" {
            continue;
        }
        let mut parts = v.split_whitespace();
        let (Some(num), Some(unit)) = (parts.next(), parts.next()) else { return 0 };
        let Ok(n) = num.replace(',', ".").parse::<f64>() else { return 0 };
        let mult = match unit {
            "B" => 1.0,
            "KiB" => 1024.0,
            "MiB" => 1024.0 * 1024.0,
            "GiB" => 1024.0 * 1024.0 * 1024.0,
            _ => return 0,
        };
        return (n * mult) as u64;
    }
    0
}

pub fn pkg_size(env: &Env, pkg: &str) -> u64 {
    match env.pm {
        Pm::Pacman => parse_installed_size(&env.run(&["pacman", "-Qi", pkg]).1),
        Pm::Dnf => {
            let (rc, out) = env.run(&["rpm", "-q", "--qf", "%{SIZE}", pkg]);
            if rc == 0 { out.trim().parse().unwrap_or(0) } else { 0 }
        }
    }
}

pub fn pkg_update(env: &Env) -> (i32, String) {
    match env.pm {
        Pm::Pacman => env.run_w(&["pacman", "-Syu", "--noconfirm"]),
        Pm::Dnf => env.run_w(&["dnf", "upgrade", "-y", "--refresh"]),
    }
}

pub fn clean_cache(env: &Env) {
    match env.pm {
        Pm::Pacman => {
            if env.have("paccache") {
                env.change_s(&["paccache", "-rk2"]);
                env.change_s(&["paccache", "-ruk0"]);
            } else {
                env.change_s(&["pacman", "-Sc", "--noconfirm"]);
            }
        }
        Pm::Dnf => {
            env.change_s(&["dnf", "clean", "all"]);
        }
    }
}

pub fn rank_mirrors(env: &Env) -> bool {
    match env.pm {
        Pm::Pacman => {
            if !env.have("pacman-mirrors") {
                env.warn("pacman-mirrors is Manjaro-specific and was not found; on Arch use reflector");
                return false;
            }
            let (mut rc, mut out) = env.run_w(&["pacman-mirrors", "--fasttrack", "10"]);
            if rc == 0 {
                (rc, out) = env.run_w(&["pacman", "-Syy"]);
            }
            if rc == 0 { env.ok("Mirrors ranked and databases refreshed") } else { env.warn(&format!("Mirror ranking failed: {}", tail(&out, 120))) }
            rc == 0
        }
        Pm::Dnf => {
            let (rc, out) = env.run_w(&["dnf", "makecache", "--refresh"]);
            if rc == 0 { env.ok("Package metadata refreshed") } else { env.warn(&format!("Refresh failed: {}", tail(&out, 120))) }
            rc == 0
        }
    }
}

pub fn flatpak_installed(env: &Env, app: &str) -> bool {
    env.have("flatpak") && env.run(&["flatpak", "info", app]).0 == 0
}

pub fn failure_tail(rc: i32, out: &str) -> String {
    let l = last_line(out);
    if l.is_empty() { format!("exit {rc}") } else { l }
}
