//! Terminal menu: numbered checklists with filter, presets and actions.

use crate::catalog::*;
use crate::env::{Env, fmt_size};
use crate::ops::*;
use crate::sys::pkg_remove;
use std::collections::BTreeSet;
use std::io::{BufRead, Write};

pub fn ask(prompt: &str) -> String {
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => "b".into(),
        Ok(_) => line.trim().to_string(),
    }
}

pub fn confirm(env: &Env, question: &str) -> bool {
    env.assume_yes || matches!(ask(&format!("{question} [y/N] ")).to_lowercase().as_str(), "y" | "yes")
}

pub enum Pick {
    Back,
    Preset(String),
    Action(String, BTreeSet<usize>),
}

/// Toggle numbers like `1 4 7-9` in a selection.
pub fn toggle_tokens(cmd: &str, len: usize, selected: &mut BTreeSet<usize>) {
    for tok in cmd.replace(',', " ").split_whitespace() {
        let flip = |n: usize, sel: &mut BTreeSet<usize>| {
            if (1..=len).contains(&n) && !sel.remove(&(n - 1)) {
                sel.insert(n - 1);
            }
        };
        if let Some((a, b)) = tok.split_once('-') {
            if let (Ok(a), Ok(b)) = (a.parse::<usize>(), b.parse::<usize>()) {
                (a..=b).for_each(|n| flip(n, selected));
            }
        } else if let Ok(n) = tok.parse::<usize>() {
            flip(n, selected);
        }
    }
}

/// Generic checklist. `render` gives (text, optional group heading) for a row.
pub fn pick<T>(title: &str, rows: &[T], render: impl Fn(&T) -> (String, Option<String>), actions: &str, presets: &[&str], initial: BTreeSet<usize>) -> Pick {
    let mut selected = initial;
    let mut flt = String::new();
    let names: Vec<&str> = actions.split(" | ").collect();
    loop {
        println!("\n== {title} ==");
        let shown: Vec<usize> = (0..rows.len()).filter(|&i| flt.is_empty() || render(&rows[i]).0.to_lowercase().contains(&flt.to_lowercase())).collect();
        let mut last_group: Option<String> = None;
        for &i in &shown {
            let (line, group) = render(&rows[i]);
            if let Some(g) = group
                && last_group.as_ref() != Some(&g) {
                    println!("\n  # {g}");
                    last_group = Some(g);
                }
            println!(" {} {:>3}. {line}", if selected.contains(&i) { "[x]" } else { "[ ]" }, i + 1);
        }
        println!("\n  numbers/ranges toggle (e.g. 1 4 7-9) | a all | n none | /text filter | {actions} | b back");
        if !presets.is_empty() {
            println!("  presets: {}", presets.iter().map(|p| format!("p {p}")).collect::<Vec<_>>().join(", "));
        }
        let cmd = ask("> ");
        match cmd.as_str() {
            "b" | "back" | "q" => return Pick::Back,
            "a" => selected = shown.iter().copied().collect(),
            "n" => selected.clear(),
            "" => flt.clear(),
            c if c.starts_with('/') => flt = c[1..].to_string(),
            c if !presets.is_empty() && c.starts_with("p ") => return Pick::Preset(c[2..].trim().to_string()),
            c if names.contains(&c) || c.split_whitespace().next().is_some_and(|w| names.contains(&w)) => {
                return Pick::Action(c.split_whitespace().next().unwrap().to_string(), selected);
            }
            c => toggle_tokens(c, rows.len(), &mut selected),
        }
    }
}

fn menu_tweaks(env: &Env) {
    let mut carry = BTreeSet::new();
    loop {
        let rows: Vec<_> = tweaks().into_iter().filter(|t| t.available(env)).collect();
        let applied: Vec<bool> = rows.iter().map(|t| t.is_applied(env)).collect();
        let idx = |t: &crate::tweaks::Tweak| rows.iter().position(|r| r.id == t.id).unwrap();
        let render = |t: &crate::tweaks::Tweak| (format!("{}  [{}]{}\n         {}", t.name, t.risk, if applied[idx(t)] { "  APPLIED" } else { "" }, t.desc), Some(t.cat.clone()));
        match pick("Tweaks", &rows, render, "apply | undo", &PRESETS, std::mem::take(&mut carry)) {
            Pick::Back => return,
            Pick::Preset(p) => {
                let ids = preset_ids(env, &p);
                carry = rows.iter().enumerate().filter(|(_, t)| ids.contains(&t.id)).map(|(i, _)| i).collect();
                println!("  preset selects {} tweaks; use apply to run them", carry.len());
            }
            Pick::Action(cmd, sel) => {
                if sel.is_empty() {
                    println!("  nothing selected");
                    continue;
                }
                if cmd == "apply" && confirm(env, &format!("Apply {} tweak(s)?", sel.len())) {
                    sel.iter().for_each(|&i| ignore(rows[i].apply(env)));
                }
                if cmd == "undo" && confirm(env, &format!("Undo {} tweak(s)?", sel.len())) {
                    sel.iter().for_each(|&i| ignore(rows[i].undo(env)));
                }
            }
        }
    }
}

fn menu_apps(env: &Env) {
    let apps = load_apps(env.pm);
    loop {
        let installed: Vec<bool> = apps.iter().map(|a| app_installed(env, a)).collect();
        let render = |a: &App| {
            let i = apps.iter().position(|x| x == a).unwrap();
            (format!("{}  ({}:{}){}", a.name, a.src, a.pkg, if installed[i] { "  INSTALLED" } else { "" }), Some(a.cat.clone()))
        };
        match pick("Install apps (P = official repo, A = AUR, F = Flatpak)", &apps, render, "install | remove | upgrade", &[], BTreeSet::new()) {
            Pick::Back | Pick::Preset(_) => return,
            Pick::Action(cmd, sel) => {
                if cmd == "upgrade" {
                    tool_update_system(env);
                    continue;
                }
                if sel.is_empty() {
                    println!("  nothing selected");
                    continue;
                }
                if cmd == "install" && confirm(env, &format!("Install {} app(s)?", sel.len())) {
                    let res: Vec<bool> = sel.iter().map(|&i| install_app(env, &apps[i])).collect();
                    let n = res.iter().filter(|&&b| b).count();
                    env.log(&format!("Install finished: {n} ok, {} failed", res.len() - n), if n == res.len() { "OK" } else { "WARN" });
                }
                if cmd == "remove" && confirm(env, &format!("Remove {} app(s)?", sel.len())) {
                    let res: Vec<bool> = sel.iter().map(|&i| remove_app(env, &apps[i])).collect();
                    let n = res.iter().filter(|&&b| b).count();
                    env.log(&format!("Remove finished: {n} ok, {} failed", res.len() - n), if n == res.len() { "OK" } else { "WARN" });
                }
            }
        }
    }
}

fn menu_cleaner(env: &Env) {
    println!("Scanning...");
    let rows = clean_targets(env.pm);
    let sizes: Vec<i64> = rows.iter().map(|t| target_size(env, t)).collect();
    let render = |t: &CleanTarget| {
        let i = rows.iter().position(|x| x.id == t.id).unwrap();
        (format!("{}  [{}]  {}", t.name, t.risk, size_label(sizes[i])), None)
    };
    if let Pick::Action(_, sel) = pick("Cleaner", &rows, render, "clean", &[], BTreeSet::new())
        && !sel.is_empty() && confirm(env, &format!("Clean {} item(s)? This cannot be undone.", sel.len())) {
            let freed: u64 = sel.iter().map(|&i| clean_target(env, &rows[i])).sum();
            env.ok(&format!("Total freed: {}", fmt_size(freed as f64)));
        }
}

fn menu_services(env: &Env) {
    loop {
        let rows = service_rows(env);
        let render = |r: &ServiceRow| (format!("{}  {}{}\n         {}: {}", r.unit, if r.enabled { "enabled" } else { "disabled" }, if r.active { " (running)" } else { "" }, r.label, r.hint), None);
        match pick("Services", &rows, render, "enable | disable | restore", &[], BTreeSet::new()) {
            Pick::Back | Pick::Preset(_) => return,
            Pick::Action(cmd, sel) => {
                for i in sel {
                    match cmd.as_str() {
                        "enable" => ignore(set_service(env, &rows[i].unit, true)),
                        "disable" => ignore(set_service(env, &rows[i].unit, false)),
                        _ => ignore(restore_service(env, &rows[i].unit)),
                    }
                }
            }
        }
    }
}

fn menu_startup(env: &Env) {
    loop {
        let rows = autostart_items(env);
        let render = |r: &StartupItem| (format!("{}  {}\n         {}", r.name, if r.enabled { "enabled" } else { "disabled" }, r.exec), Some(r.scope.to_string()));
        match pick("Startup (desktop autostart entries)", &rows, render, "disable | enable", &[], BTreeSet::new()) {
            Pick::Back | Pick::Preset(_) => return,
            Pick::Action(cmd, sel) => sel.iter().for_each(|&i| ignore(set_autostart(env, &rows[i], cmd == "enable"))),
        }
    }
}

fn menu_debloat(env: &Env) {
    let rows = installed_debloat(env);
    if rows.is_empty() {
        println!("None of the known optional packages are installed.");
        return;
    }
    let render = |r: &DebloatRow| (format!("{} ({})  {}", r.label, r.pkg, fmt_size(r.size as f64)), None);
    if let Pick::Action(_, sel) = pick("Debloat: optional packages you may not need (nothing is pre-selected)", &rows, render, "remove", &[], BTreeSet::new())
        && !sel.is_empty() && confirm(env, &format!("Remove {} package(s) with {}?", sel.len(), env.pm.remove_hint())) {
            for i in sel {
                let ok = pkg_remove(env, std::slice::from_ref(&rows[i].pkg)).0 == 0;
                env.log(&format!("{}: {}", rows[i].pkg, if ok { "removed" } else { "not removed (other packages depend on it?)" }), if ok { "OK" } else { "WARN" });
            }
        }
}

fn menu_network(env: &Env) {
    let render = |d: &&DnsProvider| (format!("{}  {}", d.label, if d.v4.is_empty() { "router default".to_string() } else { d.v4.join(", ") }), None);
    let provs: Vec<&DnsProvider> = DNS.iter().collect();
    match pick("DNS provider (applies to active NetworkManager connections)", &provs, render, "set | restore", &[], BTreeSet::new()) {
        Pick::Action(cmd, sel) if cmd == "restore" => {
            let _ = sel;
            if confirm(env, "Restore the saved DNS settings on Optimaxer-managed connections?") {
                restore_dns(env);
            }
        }
        Pick::Action(_, sel) if sel.len() == 1 => {
            let d = provs[*sel.iter().next().unwrap()];
            if confirm(env, &format!("Set DNS to {}? Optimaxer saves the current connection settings first.", d.label)) {
                set_dns(env, d.key);
            }
        }
        Pick::Action(..) => println!("  select exactly one provider"),
        _ => {}
    }
}

fn menu_tools(env: &Env) {
    let names = [
        "System information".to_string(),
        format!("Update the system ({})", env.pm.update_label()),
        env.pm.mirror_label().to_string(),
        "Show failed systemd units".into(),
        env.pm.conf_label().to_string(),
        "Disk health (SMART)".into(),
        "Undo everything Optimaxer changed".into(),
        "Check for an Optimaxer update".into(),
    ];
    loop {
        println!("\n== Tools ==");
        for (i, n) in names.iter().enumerate() {
            println!("  {}. {n}", i + 1);
        }
        let c = ask("number (b = back) > ");
        if matches!(c.as_str(), "b" | "back" | "q" | "") {
            return;
        }
        match c.parse::<usize>() {
            Ok(1) => sysinfo(env).iter().for_each(|(k, v)| println!("  {k:<10} {v}")),
            Ok(2) => ignore(tool_update_system(env)),
            Ok(3) => ignore(crate::sys::rank_mirrors(env)),
            Ok(4) => {
                let u = tool_failed_units(env);
                println!("  {}", if u.is_empty() { "no failed units".to_string() } else { u.join(", ") });
            }
            Ok(5) => {
                let f = tool_pacnew(env);
                if f.is_empty() { println!("  none") } else { f.iter().for_each(|l| println!("  {l}")) }
            }
            Ok(6) => tool_smart(env).iter().for_each(|l| println!("  {l}")),
            Ok(7) => {
                if confirm(env, &format!("Undo {} saved change(s) (tweaks, services, DNS)?", undoable_count(env))) {
                    undo_all(env);
                }
            }
            Ok(8) => ignore(self_update(env)),
            _ => {}
        }
    }
}

pub fn interactive(env: &Env) {
    let d = env.distro();
    println!("\nOptimaxer {}  -  {}", crate::env::VERSION, d.get("PRETTY_NAME").and_then(|v| v.as_str()).unwrap_or("Linux"));
    if env.dry_run {
        println!("DRY RUN: nothing will be changed.");
    }
    type Section = (&'static str, fn(&Env));
    let sections: [Section; 8] = [
        ("Install apps", menu_apps), ("Tweaks", menu_tweaks), ("Cleaner", menu_cleaner), ("Services", menu_services),
        ("Startup", menu_startup), ("Debloat packages", menu_debloat), ("Network / DNS", menu_network), ("Tools", menu_tools),
    ];
    loop {
        println!();
        for (i, (n, _)) in sections.iter().enumerate() {
            println!("  {}. {n}", i + 1);
        }
        let c = ask("section (q = quit) > ");
        if matches!(c.as_str(), "q" | "quit" | "exit" | "b") {
            return;
        }
        if let Some((_, f)) = c.parse::<usize>().ok().and_then(|n| n.checked_sub(1)).and_then(|n| sections.get(n)) {
            f(env);
        }
    }
}

fn ignore<T>(_: T) {}
