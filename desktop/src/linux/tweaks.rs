//! Tweaks backed by sysctl drop-ins, config files, line edits and systemd units; the catalog is ported from the
//! Python edition (`linux/optimaxer.py`) minus the entries that install packages or edit the bootloader.
//!
//! Applying never happens directly. `prepare_apply` and `prepare_undo` build a `Plan` and the records that go to the
//! saved-originals store; the UI shows the plan, saves the records, and only then runs it.

use super::store::{EditSnap, FileSnap, TweakRecord, UnitSnap};
use super::system::{Plan, Step, System, read_text, unit_active, unit_enabled};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Risk {
    Safe,
    Moderate,
}

impl Risk {
    pub fn label(self) -> &'static str {
        match self {
            Risk::Safe => "Safe",
            Risk::Moderate => "Moderate",
        }
    }
}

// region line edits
/// Sets `key = value` (or a bare flag when `value` is `None`) in a config file and remembers the replaced line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineEdit {
    pub path: &'static str,
    pub key: &'static str,
    pub value: Option<&'static str>,
    pub section: Option<&'static str>,
    pub sep: &'static str,
}

fn key_match(line: &str, key: &str) -> bool {
    line.trim_start()
        .strip_prefix(key)
        .is_some_and(|rest| matches!(rest.trim_start().chars().next(), None | Some('=')))
}

fn commented_match(line: &str, key: &str) -> bool {
    line.trim_start()
        .strip_prefix('#')
        .is_some_and(|rest| key_match(rest, key))
}

fn norm(line: &str) -> String {
    match line.trim().split_once('=') {
        Some((k, v)) => format!("{}={}", k.trim(), v.trim()),
        None => line.trim().to_string(),
    }
}

impl LineEdit {
    pub fn desired(&self) -> String {
        match self.value {
            None => self.key.to_string(),
            Some(v) => format!("{}{}{}", self.key, self.sep, v),
        }
    }

    pub fn is_applied_in(&self, text: &str) -> bool {
        let want = norm(&self.desired());
        text.split('\n')
            .any(|l| key_match(l, self.key) && norm(l) == want)
    }

    /// New text and the line that was replaced (`None` when a line was added).
    pub fn apply(&self, text: &str) -> (String, Option<String>) {
        let want = norm(&self.desired());
        let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
        if let Some(i) = lines.iter().position(|l| key_match(l, self.key)) {
            let original = lines[i].clone();
            if norm(&original) != want {
                lines[i] = self.desired();
            }
            return (lines.join("\n"), Some(original));
        }
        if let Some(i) = lines.iter().position(|l| commented_match(l, self.key)) {
            let original = lines[i].clone();
            lines[i] = self.desired();
            return (lines.join("\n"), Some(original));
        }
        if let Some(section) = self.section
            && let Some(i) = lines
                .iter()
                .position(|l| l.trim().eq_ignore_ascii_case(&format!("[{section}]")))
        {
            lines.insert(i + 1, self.desired());
            return (lines.join("\n"), None);
        }
        if lines.last().is_some_and(|l| l.is_empty()) {
            let at = lines.len() - 1;
            lines.insert(at, self.desired());
        } else {
            lines.push(self.desired());
        }
        (lines.join("\n"), None)
    }
}

/// Undo of one edit: put the original line back, or delete the line that was added.
pub fn revert_edit(snap: &EditSnap, text: &str) -> String {
    let want = norm(&match &snap.value {
        None => snap.key.clone(),
        Some(v) => format!("{}{}{}", snap.key, snap.sep, v),
    });
    let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
    if let Some(i) = lines
        .iter()
        .position(|l| key_match(l, &snap.key) && norm(l) == want)
    {
        match &snap.orig {
            None => {
                lines.remove(i);
            }
            Some(o) => lines[i] = o.clone(),
        }
    }
    lines.join("\n")
}
// endregion

// region catalog
pub struct Tweak {
    pub id: &'static str,
    pub name: &'static str,
    pub desc: &'static str,
    pub cat: &'static str,
    pub risk: Risk,
    pub tags: &'static [&'static str],
    pub files: Vec<(&'static str, &'static str)>,
    pub edits: Vec<LineEdit>,
    pub enable: &'static [&'static str],
    pub disable: &'static [&'static str],
    pub post: Vec<Vec<&'static str>>,
    pub undo_post: Vec<Vec<&'static str>>,
    /// Absolute paths must exist (and be readable), bare names must be on PATH.
    pub needs: &'static [&'static str],
    /// Only offered with this package manager.
    pub pm: Option<&'static str>,
}

fn tweak(
    id: &'static str,
    name: &'static str,
    desc: &'static str,
    cat: &'static str,
    risk: Risk,
    tags: &'static [&'static str],
) -> Tweak {
    Tweak {
        id,
        name,
        desc,
        cat,
        risk,
        tags,
        files: Vec::new(),
        edits: Vec::new(),
        enable: &[],
        disable: &[],
        post: Vec::new(),
        undo_post: Vec::new(),
        needs: &[],
        pm: None,
    }
}

const SYSCTL: [&str; 2] = ["sysctl", "--system"];
const SAFE_PERF: &[&str] = &["safe", "perf", "max"];

#[cfg(test)]
pub const PRESETS: [&str; 4] = ["safe", "perf", "privacy", "max"];

pub fn catalog() -> Vec<Tweak> {
    use Risk::{Moderate, Safe};
    let mut t = Vec::new();
    let mut add = |tw: Tweak| t.push(tw);

    let mut x = tweak(
        "perf-vm",
        "Memory tuning (swappiness 20, cache pressure 50)",
        "Keeps apps in RAM longer and flushes dirty pages sooner: snappier desktop, smoother large copies.",
        "Performance",
        Safe,
        SAFE_PERF,
    );
    x.files = vec![(
        "/etc/sysctl.d/99-optimaxer-vm.conf",
        "vm.swappiness = 20\nvm.vfs_cache_pressure = 50\nvm.dirty_ratio = 10\nvm.dirty_background_ratio = 5\n",
    )];
    x.post = vec![SYSCTL.to_vec()];
    add(x);

    let mut x = tweak(
        "perf-trim",
        "Weekly SSD TRIM",
        "Enables fstrim.timer so free blocks are trimmed once a week: keeps SSD speed up.",
        "Performance",
        Safe,
        SAFE_PERF,
    );
    x.enable = &["fstrim.timer"];
    x.needs = &["systemctl"];
    add(x);

    let mut x = tweak(
        "perf-iosched",
        "Best I/O scheduler per disk type",
        "NVMe: none, SATA SSD: mq-deadline, hard disk: bfq (udev rule).",
        "Performance",
        Safe,
        SAFE_PERF,
    );
    x.files = vec![(
        "/etc/udev/rules.d/60-optimaxer-ioscheduler.rules",
        "ACTION==\"add|change\", KERNEL==\"nvme[0-9]*\", ATTR{queue/scheduler}=\"none\"\nACTION==\"add|change\", KERNEL==\"sd[a-z]*|mmcblk[0-9]*\", ATTR{queue/rotational}==\"0\", ATTR{queue/scheduler}=\"mq-deadline\"\nACTION==\"add|change\", KERNEL==\"sd[a-z]\", ATTR{queue/rotational}==\"1\", ATTR{queue/scheduler}=\"bfq\"\n",
    )];
    x.post = vec![
        vec!["udevadm", "control", "--reload"],
        vec![
            "udevadm",
            "trigger",
            "--subsystem-match=block",
            "--action=change",
        ],
    ];
    add(x);

    let mut x = tweak(
        "perf-journal",
        "Limit the system journal to 200 MB",
        "Stops /var/log/journal growing to gigabytes.",
        "Performance",
        Safe,
        SAFE_PERF,
    );
    x.files = vec![(
        "/etc/systemd/journald.conf.d/99-optimaxer.conf",
        "[Journal]\nSystemMaxUse=200M\nRuntimeMaxUse=50M\n",
    )];
    x.post = vec![vec!["systemctl", "restart", "systemd-journald"]];
    add(x);

    let mut x = tweak(
        "perf-stoptimeout",
        "Faster shutdown (10 s service stop timeout)",
        "Hung services are killed after 10 s instead of 90 s.",
        "Performance",
        Safe,
        SAFE_PERF,
    );
    x.files = vec![(
        "/etc/systemd/system.conf.d/99-optimaxer.conf",
        "[Manager]\nDefaultTimeoutStopSec=10s\nDefaultTimeoutStartSec=30s\n",
    )];
    x.post = vec![vec!["systemctl", "daemon-reexec"]];
    add(x);

    let mut x = tweak(
        "perf-waitonline",
        "Do not wait for the network at boot",
        "Disables NetworkManager-wait-online.service: boots a few seconds faster. Breaks network mounts that must be ready at boot.",
        "Performance",
        Moderate,
        &["max"],
    );
    x.disable = &["NetworkManager-wait-online.service"];
    x.needs = &["/usr/lib/systemd/system/NetworkManager-wait-online.service"];
    add(x);

    let mut x = tweak(
        "perf-coredump",
        "Disable core dumps",
        "Crashing apps stop writing large dump files (privacy and disk space).",
        "Performance",
        Safe,
        &["safe", "perf", "privacy", "max"],
    );
    x.files = vec![(
        "/etc/systemd/coredump.conf.d/99-optimaxer.conf",
        "[Coredump]\nStorage=none\nProcessSizeMax=0\n",
    )];
    add(x);

    let mut x = tweak(
        "perf-wifi-powersave",
        "Wi-Fi power saving off",
        "NetworkManager keeps the Wi-Fi radio awake: lower latency and fewer dropouts, slightly more battery use on laptops.",
        "Network",
        Safe,
        &["perf", "max"],
    );
    x.files = vec![(
        "/etc/NetworkManager/conf.d/99-optimaxer-wifi.conf",
        "[connection]\nwifi.powersave = 2\n",
    )];
    x.post = vec![vec!["systemctl", "reload", "NetworkManager"]];
    x.undo_post = vec![vec!["systemctl", "reload", "NetworkManager"]];
    x.needs = &["nmcli"];
    add(x);

    let mut x = tweak(
        "perf-nmi",
        "Disable the NMI watchdog",
        "Frees a little CPU time and saves power; only used for kernel debugging.",
        "Performance",
        Safe,
        SAFE_PERF,
    );
    x.files = vec![(
        "/etc/sysctl.d/99-optimaxer-nmi.conf",
        "kernel.nmi_watchdog = 0\n",
    )];
    x.post = vec![SYSCTL.to_vec()];
    add(x);

    let mut x = tweak(
        "perf-inotify",
        "Raise inotify watch limit (524288)",
        "IDEs, file managers and sync tools stop failing with 'no space left on device' (ENOSPC) on big folders.",
        "Performance",
        Safe,
        SAFE_PERF,
    );
    x.files = vec![(
        "/etc/sysctl.d/99-optimaxer-inotify.conf",
        "fs.inotify.max_user_watches = 524288\nfs.inotify.max_user_instances = 1024\n",
    )];
    x.post = vec![SYSCTL.to_vec()];
    add(x);

    let mut x = tweak(
        "perf-mapcount",
        "Raise vm.max_map_count (1048576)",
        "Needed by many Proton/Wine games and some engines that crash with the old 65530 limit.",
        "Gaming",
        Safe,
        &["perf", "max"],
    );
    x.files = vec![(
        "/etc/sysctl.d/99-optimaxer-mapcount.conf",
        "vm.max_map_count = 1048576\n",
    )];
    x.post = vec![SYSCTL.to_vec()];
    add(x);

    let mut x = tweak(
        "perf-netbuf",
        "Larger network buffers, TCP Fast Open and MTU probing",
        "16 MB socket buffer ceilings, Fast Open for client and server, and MTU probing: better throughput on fast links and fewer stalls behind broken routers.",
        "Network",
        Safe,
        &["perf", "max"],
    );
    x.files = vec![(
        "/etc/sysctl.d/99-optimaxer-netbuf.conf",
        "net.core.rmem_max = 16777216\nnet.core.wmem_max = 16777216\nnet.core.netdev_max_backlog = 16384\nnet.ipv4.tcp_fastopen = 3\nnet.ipv4.tcp_mtu_probing = 1\nnet.ipv4.tcp_slow_start_after_idle = 0\n",
    )];
    x.post = vec![SYSCTL.to_vec()];
    add(x);

    let mut x = tweak(
        "perf-networkd-wait",
        "Faster boot: mask systemd-networkd-wait-online",
        "Stops boot waiting on systemd-networkd-wait-online; harmless on NetworkManager systems.",
        "Performance",
        Moderate,
        &["max"],
    );
    x.disable = &["systemd-networkd-wait-online.service"];
    x.needs = &["/usr/lib/systemd/system/systemd-networkd-wait-online.service"];
    add(x);

    let mut x = tweak(
        "pm-pacman",
        "pacman: parallel downloads and colour",
        "ParallelDownloads = 5 and Color in /etc/pacman.conf: much faster updates, readable output.",
        "Package manager",
        Safe,
        SAFE_PERF,
    );
    x.edits = vec![
        LineEdit {
            path: "/etc/pacman.conf",
            key: "ParallelDownloads",
            value: Some("5"),
            section: Some("options"),
            sep: " = ",
        },
        LineEdit {
            path: "/etc/pacman.conf",
            key: "Color",
            value: None,
            section: Some("options"),
            sep: " = ",
        },
    ];
    x.needs = &["/etc/pacman.conf"];
    x.pm = Some("pacman");
    add(x);

    let mut x = tweak(
        "pm-dnf",
        "dnf: parallel downloads and fastest mirror",
        "max_parallel_downloads=10 and fastestmirror=True in /etc/dnf/dnf.conf: much faster updates and installs.",
        "Package manager",
        Safe,
        SAFE_PERF,
    );
    x.edits = vec![
        LineEdit {
            path: "/etc/dnf/dnf.conf",
            key: "max_parallel_downloads",
            value: Some("10"),
            section: Some("main"),
            sep: "=",
        },
        LineEdit {
            path: "/etc/dnf/dnf.conf",
            key: "fastestmirror",
            value: Some("True"),
            section: Some("main"),
            sep: "=",
        },
    ];
    x.needs = &["/etc/dnf/dnf.conf"];
    x.pm = Some("dnf");
    add(x);

    let mut x = tweak(
        "net-bbr",
        "TCP BBR congestion control",
        "Better throughput and latency on lossy links (Wi-Fi, long distance).",
        "Network",
        Safe,
        SAFE_PERF,
    );
    x.files = vec![
        ("/etc/modules-load.d/optimaxer-bbr.conf", "tcp_bbr\n"),
        (
            "/etc/sysctl.d/99-optimaxer-net.conf",
            "net.core.default_qdisc = fq\nnet.ipv4.tcp_congestion_control = bbr\n",
        ),
    ];
    x.post = vec![vec!["modprobe", "tcp_bbr"], SYSCTL.to_vec()];
    add(x);

    let mut x = tweak(
        "sec-sysctl",
        "Kernel and network hardening",
        "Hides kernel pointers and dmesg from users, enables reverse-path filtering, ignores ICMP redirects and broadcast pings.",
        "Security",
        Safe,
        &["safe", "privacy", "max"],
    );
    x.files = vec![(
        "/etc/sysctl.d/99-optimaxer-harden.conf",
        "kernel.dmesg_restrict = 1\nkernel.kptr_restrict = 1\nnet.ipv4.conf.all.rp_filter = 1\nnet.ipv4.conf.default.rp_filter = 1\nnet.ipv4.icmp_echo_ignore_broadcasts = 1\nnet.ipv4.conf.all.accept_redirects = 0\nnet.ipv4.conf.default.accept_redirects = 0\nnet.ipv6.conf.all.accept_redirects = 0\nnet.ipv4.conf.all.send_redirects = 0\n",
    )];
    x.post = vec![SYSCTL.to_vec()];
    add(x);

    let mut x = tweak(
        "priv-avahi",
        "Disable mDNS/Avahi network discovery",
        "Stops the machine announcing itself on the LAN. Network printer and Chromecast auto-discovery stops working.",
        "Privacy",
        Moderate,
        &["privacy", "max"],
    );
    x.disable = &["avahi-daemon.service", "avahi-daemon.socket"];
    x.needs = &["/usr/lib/systemd/system/avahi-daemon.service"];
    add(x);

    t
}

pub fn by_id<'a>(tweaks: &'a [Tweak], id: &str) -> Option<&'a Tweak> {
    tweaks.iter().find(|t| t.id == id)
}

pub fn detect_pm(sys: &dyn System) -> Option<&'static str> {
    if sys.have("pacman") {
        Some("pacman")
    } else if sys.have("dnf") {
        Some("dnf")
    } else {
        None
    }
}

pub fn available(sys: &dyn System, t: &Tweak, pm: Option<&str>) -> bool {
    if t.pm.is_some_and(|wanted| Some(wanted) != pm) {
        return false;
    }
    t.needs.iter().all(|need| {
        if need.starts_with('/') {
            read_text(sys, need).is_some()
        } else {
            sys.have(need)
        }
    })
}

/// Ids of the tweaks tagged with `preset` that `is_available` accepts.
pub fn preset_ids(
    tweaks: &[Tweak],
    preset: &str,
    is_available: impl Fn(&str) -> bool,
) -> Vec<&'static str> {
    tweaks
        .iter()
        .filter(|t| t.tags.contains(&preset) && is_available(t.id))
        .map(|t| t.id)
        .collect()
}
// endregion

// region state checks
/// `key = value` lines of a sysctl drop-in.
pub fn sysctl_pairs(content: &str) -> Vec<(String, String)> {
    content
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.starts_with('#') || l.starts_with(';') {
                return None;
            }
            let (k, v) = l.split_once('=')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

fn proc_sys_path(key: &str) -> String {
    format!("/proc/sys/{}", key.replace('.', "/"))
}

fn runtime_value(sys: &dyn System, key: &str) -> Option<String> {
    read_text(sys, &proc_sys_path(key)).map(|v| v.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn is_sysctl_file(path: &str) -> bool {
    path.starts_with("/etc/sysctl.d/")
}

/// Whether the tweak is in effect: files match, edits are present, units are in the wanted state, and running sysctl
/// values agree where `/proc/sys` can be read.
pub fn is_applied(sys: &dyn System, t: &Tweak) -> bool {
    for (path, content) in &t.files {
        if read_text(sys, path).as_deref() != Some(*content) {
            return false;
        }
        if is_sysctl_file(path) {
            for (key, want) in sysctl_pairs(content) {
                let want = want.split_whitespace().collect::<Vec<_>>().join(" ");
                if runtime_value(sys, &key).is_some_and(|now| now != want) {
                    return false;
                }
            }
        }
    }
    for edit in &t.edits {
        if !read_text(sys, edit.path).is_some_and(|text| edit.is_applied_in(&text)) {
            return false;
        }
    }
    t.enable.iter().all(|u| unit_enabled(sys, u)) && t.disable.iter().all(|u| !unit_enabled(sys, u))
}
// endregion

// region plans
pub struct Prepared {
    pub plan: Plan,
    pub record: TweakRecord,
    /// Whole-file copies of edited files, saved beside the originals before the edit.
    pub backups: Vec<(String, String)>,
    pub notes: Vec<String>,
}

fn cmd(argv: &[&str]) -> Step {
    Step::run(argv)
}

/// Builds the plan that applies `t`. `previous` is the record of an earlier, possibly failed, apply: its originals are
/// kept, because the file on disk may already hold this tweak's own values.
pub fn prepare_apply(
    sys: &dyn System,
    t: &Tweak,
    previous: Option<&TweakRecord>,
    now: &str,
) -> Result<Prepared, String> {
    let mut record = previous.cloned().unwrap_or_else(|| TweakRecord {
        time: now.to_string(),
        ..TweakRecord::default()
    });
    let mut plan = Plan::default();
    let mut backups = Vec::new();
    let mut notes = Vec::new();

    for (path, content) in &t.files {
        let current = sys.read(path)?;
        if !record.files.iter().any(|f| f.path == *path) {
            record.files.push(FileSnap {
                path: path.to_string(),
                existed: current.is_some(),
                content: current.clone(),
            });
        }
        if current.as_deref() != Some(*content) {
            plan.push(Step::Write {
                path: path.to_string(),
                content: content.to_string(),
            });
        }
        if is_sysctl_file(path) {
            for (key, _) in sysctl_pairs(content) {
                if !record.sysctl.iter().any(|(k, _)| *k == key)
                    && let Some(value) = runtime_value(sys, &key)
                {
                    record.sysctl.push((key, value));
                }
            }
        }
    }

    let mut edited: Vec<(&str, String, String)> = Vec::new(); // path, original text, new text
    for edit in &t.edits {
        let slot = edited.iter().position(|(p, _, _)| *p == edit.path);
        let text = match slot {
            Some(i) => edited[i].2.clone(),
            None => match sys.read(edit.path)? {
                Some(text) => text,
                None => {
                    notes.push(format!(
                        "{} was not found, so {} is skipped.",
                        edit.path, edit.key
                    ));
                    continue;
                }
            },
        };
        let (new, orig) = edit.apply(&text);
        if !record.edits.iter().any(|e| {
            e.path == edit.path && e.key == edit.key && e.section.as_deref() == edit.section
        }) {
            record.edits.push(EditSnap {
                path: edit.path.into(),
                key: edit.key.into(),
                value: edit.value.map(Into::into),
                section: edit.section.map(Into::into),
                sep: edit.sep.into(),
                orig,
                changed: new != text,
            });
        }
        match slot {
            Some(i) => edited[i].2 = new,
            None => edited.push((edit.path, text, new)),
        }
    }
    for (path, original, new) in edited {
        if new != original {
            backups.push((path.to_string(), original));
            plan.push(Step::Write {
                path: path.to_string(),
                content: new,
            });
        }
    }

    for unit in t.enable.iter().chain(t.disable) {
        if !record.units.iter().any(|u| u.unit == *unit) {
            record.units.push(UnitSnap {
                unit: unit.to_string(),
                enabled: unit_enabled(sys, unit),
                active: unit_active(sys, unit),
            });
        }
    }
    for unit in t.enable {
        plan.push(cmd(&["systemctl", "enable", "--now", unit]));
    }
    for unit in t.disable {
        plan.push(cmd(&["systemctl", "disable", "--now", unit]));
    }
    for post in &t.post {
        plan.push(cmd(post));
    }
    Ok(Prepared {
        plan,
        record,
        backups,
        notes,
    })
}

/// Builds the plan that restores the saved originals of `t`.
pub fn prepare_undo(
    sys: &dyn System,
    t: &Tweak,
    record: &TweakRecord,
) -> Result<(Plan, Vec<String>), String> {
    let mut plan = Plan::default();
    let mut notes = Vec::new();
    // Enabled at boot and running now are independent states, including disabled but active units.
    for unit in &record.units {
        plan.push(cmd(&[
            "systemctl",
            if unit.enabled { "enable" } else { "disable" },
            &unit.unit,
        ]));
        plan.push(cmd(&[
            "systemctl",
            if unit.active { "start" } else { "stop" },
            &unit.unit,
        ]));
    }
    for file in &record.files {
        if file.existed {
            plan.push(Step::Write {
                path: file.path.clone(),
                content: file.content.clone().unwrap_or_default(),
            });
        } else {
            plan.push(Step::Remove {
                path: file.path.clone(),
            });
        }
    }
    let mut reverted: Vec<(String, String)> = Vec::new();
    for edit in record.edits.iter().filter(|e| e.changed) {
        let text = match reverted.iter().find(|(p, _)| *p == edit.path) {
            Some((_, text)) => text.clone(),
            None => sys
                .read(&edit.path)?
                .ok_or_else(|| format!("Cannot restore {}: the file is missing.", edit.path))?,
        };
        let new = revert_edit(edit, &text);
        reverted.retain(|(p, _)| *p != edit.path);
        reverted.push((edit.path.clone(), new));
    }
    for (path, content) in reverted {
        plan.push(Step::Write { path, content });
    }
    for post in &t.undo_post {
        plan.push(cmd(post));
    }
    let has = |prefix: &str| record.files.iter().any(|f| f.path.starts_with(prefix));
    if has("/etc/sysctl.d/") {
        plan.push(cmd(&SYSCTL));
        for (key, value) in &record.sysctl {
            plan.push(cmd(&["sysctl", "-w", &format!("{key}={value}")]));
        }
    }
    if has("/etc/udev/rules.d/") {
        plan.push(cmd(&["udevadm", "control", "--reload"]));
    }
    if has("/etc/systemd/system.conf.d/") {
        plan.push(cmd(&["systemctl", "daemon-reexec"]));
    }
    if has("/etc/systemd/journald.conf.d/") {
        plan.push(cmd(&["systemctl", "restart", "systemd-journald"]));
    }
    if record
        .files
        .iter()
        .any(|f| f.path.starts_with("/etc/modules-load.d/"))
    {
        notes.push("A kernel module that was loaded stays loaded until the next reboot.".into());
    }
    Ok((plan, notes))
}
// endregion

#[cfg(test)]
mod tests {
    use super::super::system::fake::FakeSystem;
    use super::*;

    fn edit(
        key: &'static str,
        value: Option<&'static str>,
        section: Option<&'static str>,
    ) -> LineEdit {
        LineEdit {
            path: "/etc/x.conf",
            key,
            value,
            section,
            sep: " = ",
        }
    }

    #[test]
    fn edits_replace_uncomment_insert_in_section_or_append() {
        let e = edit("ParallelDownloads", Some("5"), Some("options"));
        let (t, o) = e.apply("[options]\nParallelDownloads = 1\n");
        assert_eq!(t, "[options]\nParallelDownloads = 5\n");
        assert_eq!(o.as_deref(), Some("ParallelDownloads = 1"));
        let (t, o) = e.apply("#ParallelDownloads = 5\n");
        assert_eq!(t, "ParallelDownloads = 5\n");
        assert_eq!(o.as_deref(), Some("#ParallelDownloads = 5"));
        let (t, o) = e.apply("[Misc]\nA=1\n[Options]\nB=2\n");
        assert_eq!(t, "[Misc]\nA=1\n[Options]\nParallelDownloads = 5\nB=2\n");
        assert_eq!(o, None);
        let (t, o) = e.apply("A=1\n");
        assert_eq!(t, "A=1\nParallelDownloads = 5\n");
        assert_eq!(o, None);
        let (t, _) = e.apply("A=1");
        assert_eq!(t, "A=1\nParallelDownloads = 5");
    }

    #[test]
    fn an_edit_that_is_already_there_changes_nothing_and_is_detected() {
        let e = edit("Color", None, None);
        assert!(e.is_applied_in("Color\n"));
        assert!(!e.is_applied_in("#Color\n"));
        let (t, o) = e.apply("Color\n");
        assert_eq!(t, "Color\n");
        assert_eq!(o.as_deref(), Some("Color"));
        let spaced = LineEdit {
            path: "p",
            key: "k",
            value: Some("v"),
            section: None,
            sep: "=",
        };
        assert!(spaced.is_applied_in("k   =   v\n"));
        assert!(
            !spaced.is_applied_in("kk = v\n"),
            "a longer key is not the same setting"
        );
    }

    #[test]
    fn reverting_restores_the_original_line_or_removes_the_added_one() {
        let snap = |orig: Option<&str>| EditSnap {
            path: "/etc/x.conf".into(),
            key: "ParallelDownloads".into(),
            value: Some("5".into()),
            section: None,
            sep: " = ".into(),
            orig: orig.map(Into::into),
            changed: true,
        };
        assert_eq!(
            revert_edit(
                &snap(Some("#ParallelDownloads = 5")),
                "ParallelDownloads = 5\nB\n"
            ),
            "#ParallelDownloads = 5\nB\n"
        );
        assert_eq!(
            revert_edit(&snap(None), "A\nParallelDownloads = 5\nB\n"),
            "A\nB\n"
        );
        assert_eq!(
            revert_edit(&snap(None), "A\nParallelDownloads = 9\n"),
            "A\nParallelDownloads = 9\n",
            "a value changed since is left alone"
        );
    }

    #[test]
    fn catalog_ids_are_unique_and_paths_are_absolute() {
        let tweaks = catalog();
        let mut ids: Vec<_> = tweaks.iter().map(|t| t.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), tweaks.len());
        for t in &tweaks {
            assert!(
                !t.files.is_empty()
                    || !t.edits.is_empty()
                    || !t.enable.is_empty()
                    || !t.disable.is_empty(),
                "{} changes nothing",
                t.id
            );
            for (path, content) in &t.files {
                assert!(path.starts_with("/etc/"), "{path}");
                assert!(content.ends_with('\n'), "{path} should end with a newline");
            }
            assert!(t.tags.iter().all(|tag| PRESETS.contains(tag)), "{}", t.id);
            assert!(!t.desc.is_empty() && !t.name.is_empty());
        }
        assert!(
            tweaks
                .iter()
                .all(|t| t.pm.is_none_or(|p| p == "pacman" || p == "dnf"))
        );
    }

    #[test]
    fn availability_follows_files_commands_and_package_manager() {
        let tweaks = catalog();
        let sys = FakeSystem::default()
            .with_tool("systemctl")
            .with_tool("pacman")
            .with_file("/etc/pacman.conf", "[options]\n");
        let pm = detect_pm(&sys);
        assert_eq!(pm, Some("pacman"));
        assert!(available(&sys, by_id(&tweaks, "perf-trim").unwrap(), pm));
        assert!(available(&sys, by_id(&tweaks, "pm-pacman").unwrap(), pm));
        assert!(!available(&sys, by_id(&tweaks, "pm-dnf").unwrap(), pm));
        assert!(
            !available(&sys, by_id(&tweaks, "perf-wifi-powersave").unwrap(), pm),
            "needs nmcli"
        );
        assert!(
            !available(&sys, by_id(&tweaks, "priv-avahi").unwrap(), pm),
            "avahi unit file is absent"
        );
        let ids = preset_ids(&tweaks, "safe", |id| {
            by_id(&tweaks, id).is_some_and(|t| available(&sys, t, pm))
        });
        assert!(ids.contains(&"perf-vm") && !ids.contains(&"priv-avahi"));
    }

    fn apply(
        sys: &FakeSystem,
        t: &Tweak,
        previous: Option<&TweakRecord>,
    ) -> Result<Prepared, String> {
        let prepared = prepare_apply(sys, t, previous, "2026-10-31T00:00:00")?;
        sys.execute(&prepared.plan)?;
        Ok(prepared)
    }

    #[test]
    fn apply_writes_files_saves_originals_and_runs_post_commands() {
        let tweaks = catalog();
        let t = by_id(&tweaks, "perf-vm").unwrap();
        let sys = FakeSystem::default()
            .with_file("/proc/sys/vm/swappiness", "60\n")
            .with_file("/proc/sys/vm/vfs_cache_pressure", "100\n")
            .with_file("/etc/sysctl.d/99-optimaxer-vm.conf", "vm.swappiness = 10\n");
        assert!(!is_applied(&sys, t));
        let prepared = apply(&sys, t, None).unwrap();
        assert!(
            sys.file("/etc/sysctl.d/99-optimaxer-vm.conf")
                .unwrap()
                .contains("vm.swappiness = 20")
        );
        assert_eq!(
            prepared.record.files[0].content.as_deref(),
            Some("vm.swappiness = 10\n")
        );
        assert!(prepared.record.files[0].existed);
        assert!(
            prepared
                .record
                .sysctl
                .contains(&("vm.swappiness".into(), "60".into()))
        );
        assert_eq!(
            sys.executed.borrow().last(),
            Some(&Step::run(&["sysctl", "--system"]))
        );
        assert!(
            !is_applied(&sys, t),
            "the running kernel value still says 60, so the tweak is not active yet"
        );
        sys.files
            .borrow_mut()
            .insert("/proc/sys/vm/swappiness".into(), "20\n".into());
        sys.files
            .borrow_mut()
            .insert("/proc/sys/vm/vfs_cache_pressure".into(), "50\n".into());
        sys.files
            .borrow_mut()
            .insert("/proc/sys/vm/dirty_ratio".into(), "10\n".into());
        sys.files
            .borrow_mut()
            .insert("/proc/sys/vm/dirty_background_ratio".into(), "5\n".into());
        assert!(is_applied(&sys, t));
    }

    #[test]
    fn undo_restores_files_removes_new_ones_and_resets_running_values() {
        let tweaks = catalog();
        let t = by_id(&tweaks, "perf-nmi").unwrap();
        let sys = FakeSystem::default().with_file("/proc/sys/kernel/nmi_watchdog", "1\n");
        let prepared = apply(&sys, t, None).unwrap();
        assert!(!prepared.record.files[0].existed);
        let (plan, _) = prepare_undo(&sys, t, &prepared.record).unwrap();
        assert!(plan.steps.contains(&Step::Remove {
            path: "/etc/sysctl.d/99-optimaxer-nmi.conf".into()
        }));
        assert!(
            plan.steps
                .contains(&Step::run(&["sysctl", "-w", "kernel.nmi_watchdog=1"]))
        );
        sys.execute(&plan).unwrap();
        assert_eq!(sys.file("/etc/sysctl.d/99-optimaxer-nmi.conf"), None);
    }

    #[test]
    fn units_are_snapshotted_and_restored_independently() {
        let tweaks = catalog();
        let t = by_id(&tweaks, "perf-trim").unwrap();
        let sys = FakeSystem::default().with_unit("fstrim.timer", false, true);
        let prepared = apply(&sys, t, None).unwrap();
        assert_eq!(
            prepared.record.units,
            vec![UnitSnap {
                unit: "fstrim.timer".into(),
                enabled: false,
                active: true
            }]
        );
        assert!(is_applied(&sys, t));
        let (plan, _) = prepare_undo(&sys, t, &prepared.record).unwrap();
        sys.execute(&plan).unwrap();
        assert!(!unit_enabled(&sys, "fstrim.timer"));
        assert!(
            unit_active(&sys, "fstrim.timer"),
            "disabled but still running, as it was"
        );
    }

    #[test]
    fn disable_tweaks_turn_units_off_and_back_on() {
        let tweaks = catalog();
        let t = by_id(&tweaks, "priv-avahi").unwrap();
        let sys = FakeSystem::default()
            .with_unit("avahi-daemon.service", true, true)
            .with_unit("avahi-daemon.socket", true, true);
        let prepared = apply(&sys, t, None).unwrap();
        assert!(is_applied(&sys, t));
        assert!(!sys.active.borrow().contains("avahi-daemon.service"));
        let (plan, _) = prepare_undo(&sys, t, &prepared.record).unwrap();
        sys.execute(&plan).unwrap();
        assert!(
            sys.enabled.borrow().contains("avahi-daemon.socket")
                && sys.active.borrow().contains("avahi-daemon.service")
        );
    }

    #[test]
    fn line_edit_tweaks_change_only_their_line_and_undo_keeps_other_edits() {
        let tweaks = catalog();
        let t = by_id(&tweaks, "pm-pacman").unwrap();
        let original = "[options]\n#ParallelDownloads = 5\nHoldPkg = pacman glibc\n";
        let sys = FakeSystem::default().with_file("/etc/pacman.conf", original);
        let prepared = apply(&sys, t, None).unwrap();
        assert_eq!(
            prepared.backups,
            vec![("/etc/pacman.conf".to_string(), original.to_string())]
        );
        assert_eq!(
            sys.file("/etc/pacman.conf").unwrap(),
            "[options]\nColor\nParallelDownloads = 5\nHoldPkg = pacman glibc\n"
        );
        assert!(is_applied(&sys, t));
        // The user edits another line afterwards; undo must not revert that.
        let edited = sys
            .file("/etc/pacman.conf")
            .unwrap()
            .replace("HoldPkg = pacman glibc", "HoldPkg = pacman");
        sys.files
            .borrow_mut()
            .insert("/etc/pacman.conf".into(), edited);
        let (plan, _) = prepare_undo(&sys, t, &prepared.record).unwrap();
        sys.execute(&plan).unwrap();
        assert_eq!(
            sys.file("/etc/pacman.conf").unwrap(),
            "[options]\n#ParallelDownloads = 5\nHoldPkg = pacman\n"
        );
    }

    #[test]
    fn a_second_apply_keeps_the_first_originals() {
        let tweaks = catalog();
        let t = by_id(&tweaks, "perf-journal").unwrap();
        let sys = FakeSystem::default().with_file(
            "/etc/systemd/journald.conf.d/99-optimaxer.conf",
            "[Journal]\nSystemMaxUse=1G\n",
        );
        *sys.fail_prefix.borrow_mut() = Some("systemctl restart".into());
        let first = prepare_apply(&sys, t, None, "t").unwrap();
        assert!(
            sys.execute(&first.plan).is_err(),
            "the restart failed after the file was written"
        );
        let second = prepare_apply(&sys, t, Some(&first.record), "later").unwrap();
        assert_eq!(
            second.record.files[0].content.as_deref(),
            Some("[Journal]\nSystemMaxUse=1G\n")
        );
        assert_eq!(second.record.time, "t");
        assert!(
            second
                .plan
                .steps
                .iter()
                .all(|s| !matches!(s, Step::Write { .. })),
            "nothing left to write"
        );
    }

    #[test]
    fn unreadable_files_refuse_the_tweak_instead_of_losing_the_original() {
        let tweaks = catalog();
        let t = by_id(&tweaks, "perf-coredump").unwrap();
        let sys = FakeSystem::default();
        sys.unreadable
            .borrow_mut()
            .insert("/etc/systemd/coredump.conf.d/99-optimaxer.conf".into());
        let error = prepare_apply(&sys, t, None, "t").err().unwrap();
        assert!(error.contains("permission denied"));
    }

    #[test]
    fn missing_edit_targets_are_skipped_with_a_note() {
        let tweaks = catalog();
        let t = by_id(&tweaks, "pm-dnf").unwrap();
        let sys = FakeSystem::default();
        let prepared = prepare_apply(&sys, t, None, "t").unwrap();
        assert!(prepared.plan.is_empty());
        assert_eq!(prepared.notes.len(), 2);
    }

    #[test]
    fn a_cancelled_authentication_changes_nothing() {
        let tweaks = catalog();
        let t = by_id(&tweaks, "perf-nmi").unwrap();
        let sys = FakeSystem::default();
        *sys.refuse.borrow_mut() = true;
        let prepared = prepare_apply(&sys, t, None, "t").unwrap();
        let error = sys.execute(&prepared.plan).unwrap_err();
        assert!(error.contains("Nothing was changed"));
        assert_eq!(sys.file("/etc/sysctl.d/99-optimaxer-nmi.conf"), None);
    }
}
