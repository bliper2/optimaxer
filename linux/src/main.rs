//! Optimaxer for Manjaro / Arch Linux and Fedora: tweaks with undo, app installer, cleaner, services, startup, debloat, DNS and tools.
//!
//! Everything that changes the system records what it changed in /var/lib/optimaxer/state.json so it can be undone.
//! Use --dry-run to see every command and file change without making any.

mod catalog;
mod data;
mod env;
#[cfg(feature = "gui")]
mod gui;
mod menu;
mod ops;
mod sys;
#[cfg(test)]
mod tests;
mod tweaks;

use catalog::*;
use clap::{Parser, Subcommand};
use env::{Env, Pm, VERSION, detect_pm, fmt_size, is_root};
use ops::*;
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode};

#[derive(Parser)]
#[command(name = "optimaxer", version = VERSION, about = "Optimaxer for Manjaro/Arch Linux and Fedora")]
pub struct Cli {
    /// show what would change without changing anything
    #[arg(long, global = true)]
    dry_run: bool,
    /// do not ask for confirmation
    #[arg(short = 'y', long, global = true)]
    yes: bool,
    /// with no command: open the terminal menu instead of the window
    #[arg(long)]
    text: bool,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Clone, Copy, clap::ValueEnum, PartialEq)]
enum Act {
    List,
    Apply,
    Undo,
}

#[derive(Clone, Copy, clap::ValueEnum, PartialEq)]
enum AppAct {
    List,
    Install,
    Remove,
}

#[derive(Clone, Copy, clap::ValueEnum, PartialEq)]
enum CleanAct {
    Scan,
    Run,
}

#[derive(Subcommand)]
enum Cmd {
    /// list/apply/undo tweaks
    Tweaks {
        action: Act,
        ids: Vec<String>,
        #[arg(long, value_parser = ["safe", "perf", "privacy", "max"])]
        preset: Option<String>,
        /// with undo: every applied tweak
        #[arg(long)]
        all: bool,
    },
    /// list/install/remove apps
    Apps {
        action: AppAct,
        /// package names (e.g. firefox brave-bin)
        names: Vec<String>,
    },
    /// scan or clean
    Clean { action: CleanAct, ids: Vec<String> },
    /// list notable services
    Services,
    /// list autostart entries
    Startup,
    /// list removable optional packages
    Debloat,
    /// set DNS provider (or `restore`)
    Dns { provider: String },
    /// apply a saved selection (exported from the GUI)
    Config {
        #[arg(value_parser = ["apply"])]
        action: String,
        file: String,
    },
    /// add Optimaxer to the application menu (and Desktop)
    Launcher,
    /// open the graphical interface
    Gui,
    /// update Optimaxer itself
    Update,
    /// system information
    Info,
}

pub fn has_display() -> bool {
    std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty()) || std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
}

fn exe() -> std::path::PathBuf {
    std::env::current_exe().unwrap_or_else(|_| "optimaxer".into())
}

/// Re-run this program as root. A window is opened as the user first, then elevated with the display variables kept.
fn elevate(env: &Env, args: &[String], gui: bool) -> ExitCode {
    if gui {
        let mut keep: Vec<String> = ["DISPLAY", "XAUTHORITY", "WAYLAND_DISPLAY", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS"]
            .iter()
            .filter_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()).map(|v| format!("{k}={v}")))
            .collect();
        keep.push(format!("SUDO_USER={}", std::env::var("USER").unwrap_or_else(|_| "root".into())));
        if env.have("pkexec") {
            eprintln!("Optimaxer needs administrator rights; asking for your password...");
            let err = Command::new("pkexec").arg("env").args(&keep).arg(exe()).args(args).exec();
            eprintln!("pkexec failed: {err}");
        }
        if env.have("sudo") {
            let err = Command::new("sudo").arg("-E").arg("env").args(&keep).arg(exe()).args(args).exec();
            eprintln!("sudo failed: {err}");
        }
        eprintln!("Optimaxer needs root: install polkit (pkexec) or sudo, or start it with --dry-run.");
        return ExitCode::from(1);
    }
    if !env.have("sudo") {
        eprintln!("Optimaxer needs root: run it with sudo (or use --dry-run).");
        return ExitCode::from(1);
    }
    eprintln!("Optimaxer needs administrator rights for system changes; asking sudo...");
    let err = Command::new("sudo").arg("-E").arg(exe()).args(args).exec();
    eprintln!("sudo failed: {err}");
    ExitCode::from(1)
}

/// Once, in a terminal: offer to add Optimaxer to the app menu and the command line. The answer is remembered.
fn first_run_offer(env: &Env) {
    use std::io::IsTerminal;
    let mut settings = load_settings(env);
    if settings.get("setup_offered").and_then(|v| v.as_bool()).unwrap_or(false) || !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        return;
    }
    if std::fs::canonicalize(installed_binary(env)).ok() == std::fs::canonicalize(exe()).ok() {
        return;
    }
    let ans = menu::ask("Add Optimaxer to the application menu and install an 'optimaxer' command so it opens without this file? [Y/n] ").to_lowercase();
    settings.insert("setup_offered".into(), true.into());
    save_settings(env, &settings);
    if ans == "n" || ans == "no" || ans == "b" {
        println!("Okay. Run  optimaxer launcher  later if you change your mind.");
        return;
    }
    match install_command(env).and_then(|mut a| create_launcher(env).map(|b| {
        a.extend(b);
        a
    })) {
        Ok(made) => {
            println!("Done. Open Optimaxer from the application menu, or type  optimaxer  in a terminal.");
            let bin = format!("{}/.local/bin", env.real_home());
            if !std::env::var("PATH").unwrap_or_default().split(':').any(|p| p == bin) {
                println!("If 'optimaxer' is not found, log out and in once so ~/.local/bin is on your PATH.");
            }
            made.iter().for_each(|p| println!("  {p}"));
        }
        Err(e) => eprintln!("Setup failed: {e}"),
    }
}

fn main() -> ExitCode {
    // behave like other command-line tools when the reader goes away (`optimaxer apps list | head`)
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let cli = Cli::parse();
    let mut env = Env::new(Pm::Pacman);
    env.dry_run = cli.dry_run;
    env.assume_yes = cli.yes;
    match detect_pm(&env) {
        Some(pm) => env.pm = pm,
        None => {
            eprintln!("This edition supports Manjaro and other Arch-based distributions (pacman) and Fedora and other dnf-based distributions.");
            return ExitCode::from(2);
        }
    }
    let mut cmd = cli.cmd;
    let gui_built = cfg!(feature = "gui");
    if cmd.is_none() && !cli.text && has_display() && gui_built {
        // plain `optimaxer` on a desktop opens the window; --text keeps the terminal menu
        cmd = Some(Cmd::Gui);
    }
    if matches!(cmd, None | Some(Cmd::Gui)) && !env.dry_run && !is_root() {
        first_run_offer(&env);
    }
    let mutating = match &cmd {
        None => true,
        Some(Cmd::Tweaks { action, .. }) => *action != Act::List,
        Some(Cmd::Apps { action, .. }) => *action != AppAct::List,
        Some(Cmd::Clean { action, .. }) => *action != CleanAct::Scan,
        Some(Cmd::Dns { .. } | Cmd::Config { .. }) => true,
        _ => false,
    };
    if matches!(cmd, Some(Cmd::Gui)) {
        if !gui_built {
            eprintln!("This build has no graphical interface; use the terminal menu: optimaxer --text");
            return ExitCode::from(2);
        }
        if !has_display() {
            eprintln!("No display available. Use the text menu: optimaxer --text");
            return ExitCode::from(2);
        }
        if !env.dry_run && !is_root() {
            return elevate(&env, &argv, true);
        }
    } else if mutating && !env.dry_run && !is_root() {
        return elevate(&env, &argv, false);
    }

    let rc = match cmd {
        #[cfg(feature = "gui")]
        Some(Cmd::Gui) => gui::run(env),
        #[cfg(not(feature = "gui"))]
        Some(Cmd::Gui) => 2,
        Some(Cmd::Launcher) => match install_command(&env).and_then(|mut a| create_launcher(&env).map(|b| {
            a.extend(b);
            a
        })) {
            Ok(made) => {
                made.iter().for_each(|p| println!("Created {p}"));
                0
            }
            Err(e) => {
                eprintln!("Setup failed: {e}");
                1
            }
        },
        Some(Cmd::Config { file, .. }) => match std::fs::read_to_string(&file).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<ConfigFile>(&t).map_err(|e| e.to_string())) {
            Ok(cfg) => i32::from(!apply_config(&env, &cfg)),
            Err(e) => {
                eprintln!("Cannot read {file}: {e}");
                2
            }
        },
        None => {
            menu::interactive(&env);
            0
        }
        Some(Cmd::Info) => {
            sysinfo(&env).iter().for_each(|(k, v)| println!("{k:<10} {v}"));
            0
        }
        Some(Cmd::Tweaks { action, ids, preset, all }) => run_tweaks(&env, action, ids, preset, all),
        Some(Cmd::Apps { action, names }) => run_apps(&env, action, names),
        Some(Cmd::Clean { action, ids }) => run_clean(&env, action, &ids),
        Some(Cmd::Services) => {
            for r in service_rows(&env) {
                println!("{:<38} {} {}", r.unit, if r.enabled { "enabled " } else { "disabled" }, r.label);
            }
            0
        }
        Some(Cmd::Startup) => {
            for r in autostart_items(&env) {
                println!("{} {:<6} {}", if r.enabled { "on " } else { "off" }, r.scope, r.name);
            }
            0
        }
        Some(Cmd::Debloat) => {
            for r in installed_debloat(&env) {
                println!("{:<22} {:>10}  {}", r.pkg, fmt_size(r.size as f64), r.label);
            }
            0
        }
        Some(Cmd::Dns { provider }) => run_dns(&env, &provider),
        Some(Cmd::Update) => i32::from(!self_update(&env)),
    };
    ExitCode::from(rc as u8)
}

fn run_tweaks(env: &Env, action: Act, ids: Vec<String>, preset: Option<String>, all: bool) -> i32 {
    if action == Act::List {
        for t in tweaks().iter().filter(|t| t.available(env)) {
            println!("{} {:<18} [{}] {}", if t.is_applied(env) { '*' } else { ' ' }, t.id, t.risk, t.name);
        }
        return 0;
    }
    let mut ids = ids;
    if let Some(p) = preset {
        ids.extend(preset_ids(env, &p));
    }
    if action == Act::Undo && all {
        match env.load_state() {
            Ok(st) => ids = st.keys().filter(|k| tweak_by_id(k).is_some()).cloned().collect(),
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        }
    }
    if ids.is_empty() {
        eprintln!("No tweaks given: pass ids, --preset, or --all (undo).");
        return 2;
    }
    let mut bad = 0;
    for id in ids {
        match tweak_by_id(&id) {
            None => {
                env.warn(&format!("Unknown tweak {id}"));
                bad += 1;
            }
            Some(t) => {
                if !(if action == Act::Apply { t.apply(env) } else { t.undo(env) }) {
                    bad += 1;
                }
            }
        }
    }
    i32::from(bad > 0)
}

fn run_apps(env: &Env, action: AppAct, names: Vec<String>) -> i32 {
    let apps = load_apps(env.pm);
    if action == AppAct::List {
        for a in &apps {
            println!("{} {:<28} ({}) {}", if app_installed(env, a) { '*' } else { ' ' }, a.pkg, a.src, a.name);
        }
        return 0;
    }
    let mut ok = true;
    for n in names {
        let app = apps.iter().find(|a| a.pkg == n).cloned().unwrap_or(App { cat: String::new(), name: n.clone(), src: 'P', pkg: n });
        ok &= if action == AppAct::Install { install_app(env, &app) } else { remove_app(env, &app) };
    }
    i32::from(!ok)
}

fn run_clean(env: &Env, action: CleanAct, ids: &[String]) -> i32 {
    let mut targets: Vec<CleanTarget> = clean_targets(env.pm).into_iter().filter(|t| ids.is_empty() || ids.iter().any(|i| i == t.id)).collect();
    if action == CleanAct::Scan {
        for t in &targets {
            println!("{:<16} {:>10}  {}", t.id, size_label(target_size(env, t)), t.name);
        }
    } else {
        if ids.is_empty() {
            targets.retain(|t| t.risk == "safe");
        }
        let freed: u64 = targets.iter().map(|t| clean_target(env, t)).sum();
        println!("Freed: {}", fmt_size(freed as f64));
    }
    0
}

fn run_dns(env: &Env, provider: &str) -> i32 {
    if provider != "restore" && dns_provider(provider).is_none() {
        eprintln!("Unknown DNS provider {provider}; choose one of: {}, restore", DNS.iter().map(|d| d.key).collect::<Vec<_>>().join(", "));
        return 2;
    }
    let question = if provider == "restore" {
        "Restore the saved DNS settings?".to_string()
    } else {
        format!("Set DNS to {}? The current connection settings are backed up first.", dns_provider(provider).unwrap().label)
    };
    if !menu::confirm(env, &question) {
        return 1;
    }
    i32::from(!(if provider == "restore" { restore_dns(env) } else { set_dns(env, provider) }))
}
