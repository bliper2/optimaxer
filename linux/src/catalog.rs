//! The tweak catalog, apps, debloat and cleaner targets.

use crate::data::*;
use crate::env::{Env, Pm};
use crate::sys::{flatpak_installed, orphans, pkg_installed, pkg_size};
use crate::tweaks::{LineEdit, Tweak};
use serde_json::{Map, Value, json};

pub const PRESETS: [&str; 4] = ["safe", "perf", "privacy", "max"];

const SYSCTL: &[&str] = &["sysctl", "--system"];

/// Regenerates the GRUB menu on Debian-style, Arch-style and Fedora-style systems.
pub const GRUB_REGEN: &str = "command -v update-grub >/dev/null && update-grub || { command -v grub2-mkconfig >/dev/null && grub2-mkconfig -o /etc/grub2.cfg || grub-mkconfig -o /boot/grub/grub.cfg; }";

/// JSON text for a browser managed-policy file. Firefox wraps the policies in a top-level "policies" key.
fn policy(policies: &Value, wrap: bool) -> String {
    let v = if wrap { json!({ "policies": policies }) } else { policies.clone() };
    serde_json::to_string_pretty(&v).unwrap() + "\n"
}

fn merge(base: &Value, extra: Value) -> Value {
    let mut m: Map<String, Value> = base.as_object().unwrap().clone();
    for (k, v) in extra.as_object().unwrap() {
        m.insert(k.clone(), v.clone());
    }
    Value::Object(m)
}

/// Policies understood by every Chromium-based browser (Chrome, Chromium, Brave, Edge). A browser ignores keys it does not know.
pub fn chromium_privacy() -> Value {
    json!({
        "MetricsReportingEnabled": false, "SafeBrowsingExtendedReportingEnabled": false, "UrlKeyedAnonymizedDataCollectionEnabled": false,
        "SpellCheckServiceEnabled": false, "AlternateErrorPagesEnabled": false, "SearchSuggestEnabled": false, "BackgroundModeEnabled": false,
        "PromotionalTabsEnabled": false, "ShoppingListEnabled": false, "DefaultBrowserSettingEnabled": false, "NetworkPredictionOptions": 2,
        "PrivacySandboxPromptEnabled": false, "PrivacySandboxAdTopicsEnabled": false, "PrivacySandboxSiteEnabledAdsEnabled": false,
        "PrivacySandboxAdMeasurementEnabled": false, "WebRtcIPHandlingPolicy": "default_public_interface_only",
        "GenAiDefaultSettings": 2, "HelpMeWriteSettings": 2, "TabCompareSettings": 2, "HistorySearchSettings": 2, "GeminiSettings": 1
    })
}

pub fn brave_policy() -> Value {
    merge(&chromium_privacy(), json!({
        "BraveRewardsDisabled": true, "BraveWalletDisabled": true, "BraveVPNDisabled": true, "BraveAIChatEnabled": false,
        "BraveNewsDisabled": true, "BraveTalkDisabled": true, "BraveP3AEnabled": false, "BraveStatsPingEnabled": false, "BraveWebDiscoveryEnabled": false
    }))
}

pub fn edge_policy() -> Value {
    merge(&chromium_privacy(), json!({
        "DiagnosticData": 0, "EdgeShoppingAssistantEnabled": false, "HubsSidebarEnabled": false, "EdgeFollowEnabled": false,
        "ShowRecommendationsEnabled": false, "PersonalizationReportingEnabled": false, "StartupBoostEnabled": false, "NewTabPageContentEnabled": false,
        "SpotlightExperiencesAndRecommendationsEnabled": false, "EdgeWorkspacesEnabled": false, "ShowMicrosoftRewards": false, "UserFeedbackAllowed": false,
        "Microsoft365CopilotChatIconEnabled": false
    }))
}

pub fn firefox_policy() -> Value {
    let prefs = [
        "browser.ml.chat.enabled", "browser.ml.chat.sidebar", "browser.ml.linkPreview.enabled", "browser.tabs.groups.smart.enabled",
        "datareporting.healthreport.uploadEnabled", "app.shield.optoutstudies.enabled", "browser.newtabpage.activity-stream.feeds.telemetry",
        "browser.newtabpage.activity-stream.telemetry", "browser.discovery.enabled", "browser.newtabpage.activity-stream.showWeather",
        "browser.newtabpage.activity-stream.system.showWeather", "browser.newtabpage.activity-stream.showSponsored",
        "browser.newtabpage.activity-stream.showSponsoredTopSites", "browser.newtabpage.activity-stream.feeds.section.topstories",
        "browser.newtabpage.activity-stream.feeds.system.topstories", "browser.newtabpage.activity-stream.widgets.system.enabled",
        "browser.newtabpage.activity-stream.widgets.enabled", "browser.newtabpage.activity-stream.feeds.weatherfeed",
        "browser.newtabpage.activity-stream.discoverystream.sponsoredCollections.enabled",
    ];
    let mut p = Map::new();
    for k in prefs {
        p.insert(k.to_string(), json!({"Value": false, "Status": "default"}));
    }
    json!({
        "DisableTelemetry": true, "DisableFirefoxStudies": true, "DisablePocket": true, "DisableFeedbackCommands": true, "DontCheckDefaultBrowser": true,
        "OverrideFirstRunPage": "", "OverridePostUpdatePage": "",
        "FirefoxSuggest": {"WebSuggestions": false, "SponsoredSuggestions": false, "ImproveSuggest": false},
        "FirefoxHome": {"SponsoredTopSites": false, "SponsoredPocket": false, "Pocket": false, "Stories": false, "SponsoredStories": false, "Snippets": false},
        "UserMessaging": {"ExtensionRecommendations": false, "FeatureRecommendations": false, "UrlbarInterventions": false, "SkipOnboarding": true, "MoreFromMozilla": false},
        "EnableTrackingProtection": {"Value": true, "Cryptomining": true, "Fingerprinting": true, "EmailTracking": true},
        "Preferences": Value::Object(p)
    })
}

// Firefox's "Preferences" policy only accepts a short allow-list of settings and silently ignores the rest (the new-tab widgets, weather and
// sponsored stories are not on it). An autoconfig file has no such limit, so the new-tab and telemetry settings are locked this way.
fn ff_cfg() -> String {
    let newtab = [
        "showSponsored", "showSponsoredTopSites", "system.showSponsored", "showWeather", "system.showWeather", "feeds.weatherfeed",
        "feeds.section.topstories", "feeds.system.topstories", "widgets.enabled", "widgets.system.enabled", "widgets.lists.enabled",
        "widgets.focusTimer.enabled", "discoverystream.enabled", "discoverystream.sponsoredCollections.enabled", "feeds.telemetry", "telemetry",
    ];
    let mut locked: Vec<String> = newtab.iter().map(|n| format!("browser.newtabpage.activity-stream.{n}")).collect();
    locked.extend(
        [
            "browser.ml.chat.enabled", "browser.ml.chat.sidebar", "browser.ml.linkPreview.enabled", "browser.tabs.groups.smart.enabled",
            "extensions.pocket.enabled", "app.normandy.enabled", "app.shield.optoutstudies.enabled", "toolkit.telemetry.enabled",
            "toolkit.telemetry.unified", "toolkit.telemetry.archive.enabled", "datareporting.healthreport.uploadEnabled",
            "datareporting.policy.dataSubmissionEnabled", "browser.discovery.enabled", "browser.urlbar.suggest.quicksuggest.sponsored",
            "browser.urlbar.suggest.quicksuggest.nonsponsored",
        ]
        .map(String::from),
    );
    let mut out = String::from("// Optimaxer: locked Firefox preferences. Undo the tweak in Optimaxer to remove this file.\n");
    for k in locked {
        out.push_str(&format!("lockPref(\"{k}\", false);\n"));
    }
    out
}

const FF_AUTOCONFIG: &str = "pref(\"general.config.filename\", \"optimaxer.cfg\");\npref(\"general.config.obscure_value\", 0);\n";
const POLICY_NOTE: &str = " The browser shows 'managed by your organization'; any policy file you already have is saved and restored on undo.";

pub fn tweaks() -> Vec<Tweak> {
    let fx = firefox_policy();
    let chromium = chromium_privacy();
    let t = Tweak::new;
    let sysctl_post: &[&[&str]] = &[SYSCTL];
    vec![
        // ---- performance
        t("perf-vm", "Memory tuning (swappiness 20, cache pressure 50)", "Keeps apps in RAM longer and flushes dirty pages sooner: snappier desktop, smoother large copies.", "Performance")
            .tags(&["safe", "perf", "max"])
            .file("/etc/sysctl.d/99-optimaxer-vm.conf", "vm.swappiness = 20\nvm.vfs_cache_pressure = 50\nvm.dirty_ratio = 10\nvm.dirty_background_ratio = 5\n")
            .post(sysctl_post),
        t("perf-zram", "Compressed swap in RAM (zram)", "Creates a zstd-compressed swap device sized min(RAM/2, 8 GB): fewer freezes under memory pressure and less SSD wear. Installs zram-generator.", "Performance")
            .tags(&["safe", "perf", "max"]).pkgs(&["zram-generator"])
            .file("/etc/systemd/zram-generator.conf", "[zram0]\nzram-size = min(ram / 2, 8192)\ncompression-algorithm = zstd\nswap-priority = 100\n")
            .file("/etc/sysctl.d/99-optimaxer-zram.conf", "vm.swappiness = 100\nvm.page-cluster = 0\n")
            .post(&[&["systemctl", "daemon-reload"], &["systemctl", "start", "systemd-zram-setup@zram0.service"], SYSCTL])
            .undo_post(&[&["systemctl", "stop", "systemd-zram-setup@zram0.service"], &["systemctl", "daemon-reload"]]),
        t("perf-trim", "Weekly SSD TRIM", "Enables fstrim.timer so free blocks are trimmed once a week: keeps SSD speed up.", "Performance")
            .tags(&["safe", "perf", "max"]).enable(&["fstrim.timer"]).needs(&["systemctl"]),
        t("perf-iosched", "Best I/O scheduler per disk type", "NVMe: none, SATA SSD: mq-deadline, hard disk: bfq (udev rule).", "Performance").tags(&["safe", "perf", "max"])
            .file("/etc/udev/rules.d/60-optimaxer-ioscheduler.rules",
                "ACTION==\"add|change\", KERNEL==\"nvme[0-9]*\", ATTR{queue/scheduler}=\"none\"\n\
                 ACTION==\"add|change\", KERNEL==\"sd[a-z]*|mmcblk[0-9]*\", ATTR{queue/rotational}==\"0\", ATTR{queue/scheduler}=\"mq-deadline\"\n\
                 ACTION==\"add|change\", KERNEL==\"sd[a-z]\", ATTR{queue/rotational}==\"1\", ATTR{queue/scheduler}=\"bfq\"\n")
            .post(&[&["udevadm", "control", "--reload"], &["udevadm", "trigger", "--subsystem-match=block", "--action=change"]]),
        t("perf-journal", "Limit the system journal to 200 MB", "Stops /var/log/journal growing to gigabytes.", "Performance").tags(&["safe", "perf", "max"])
            .file("/etc/systemd/journald.conf.d/99-optimaxer.conf", "[Journal]\nSystemMaxUse=200M\nRuntimeMaxUse=50M\n")
            .post(&[&["systemctl", "restart", "systemd-journald"]]),
        t("perf-stoptimeout", "Faster shutdown (10 s service stop timeout)", "Hung services are killed after 10 s instead of 90 s.", "Performance").tags(&["safe", "perf", "max"])
            .file("/etc/systemd/system.conf.d/99-optimaxer.conf", "[Manager]\nDefaultTimeoutStopSec=10s\nDefaultTimeoutStartSec=30s\n")
            .post(&[&["systemctl", "daemon-reexec"]]),
        t("perf-waitonline", "Do not wait for the network at boot", "Disables NetworkManager-wait-online.service: boots a few seconds faster. Breaks network mounts that must be ready at boot.", "Performance")
            .risk("moderate").tags(&["max"]).disable(&["NetworkManager-wait-online.service"]).needs(&["/usr/lib/systemd/system/NetworkManager-wait-online.service"]),
        t("perf-earlyoom", "Early out-of-memory killer", "earlyoom kills the biggest app before the system freezes completely when RAM and swap run out.", "Performance")
            .tags(&["safe", "perf", "max"]).pkgs(&["earlyoom"]).enable(&["earlyoom.service"]),
        t("perf-coredump", "Disable core dumps", "Crashing apps stop writing large dump files (privacy and disk space).", "Performance").tags(&["safe", "perf", "privacy", "max"])
            .file("/etc/systemd/coredump.conf.d/99-optimaxer.conf", "[Coredump]\nStorage=none\nProcessSizeMax=0\n"),
        t("perf-wifi-powersave", "Wi-Fi power saving off", "NetworkManager keeps the Wi-Fi radio awake: lower latency and fewer dropouts, slightly more battery use on laptops.", "Network")
            .tags(&["perf", "max"]).needs(&["nmcli"])
            .file("/etc/NetworkManager/conf.d/99-optimaxer-wifi.conf", "[connection]\nwifi.powersave = 2\n")
            .post(&[&["systemctl", "reload", "NetworkManager"]]).undo_post(&[&["systemctl", "reload", "NetworkManager"]]),
        t("perf-nmi", "Disable the NMI watchdog", "Frees a little CPU time and saves power; only used for kernel debugging.", "Performance").tags(&["safe", "perf", "max"])
            .file("/etc/sysctl.d/99-optimaxer-nmi.conf", "kernel.nmi_watchdog = 0\n").post(sysctl_post),
        t("perf-grubtimeout", "Shorter GRUB menu timeout (2 s)", "Boots 3 seconds sooner. The menu is still reachable by holding Shift/Esc.", "Performance").risk("moderate").tags(&["max"])
            .edit(LineEdit::new("/etc/default/grub", "GRUB_TIMEOUT", Some("2"), None, "=")).needs(&["/etc/default/grub"])
            .post(&[&["sh", "-c", GRUB_REGEN]]).undo_post(&[&["sh", "-c", GRUB_REGEN]]),
        t("perf-inotify", "Raise inotify watch limit (524288)", "IDEs, file managers and sync tools stop failing with 'no space left on device' (ENOSPC) on big folders.", "Performance")
            .tags(&["safe", "perf", "max"]).file("/etc/sysctl.d/99-optimaxer-inotify.conf", "fs.inotify.max_user_watches = 524288\nfs.inotify.max_user_instances = 1024\n").post(sysctl_post),
        t("perf-mapcount", "Raise vm.max_map_count (1048576)", "Needed by many Proton/Wine games and some engines that crash with the old 65530 limit.", "Gaming")
            .tags(&["perf", "max"]).file("/etc/sysctl.d/99-optimaxer-mapcount.conf", "vm.max_map_count = 1048576\n").post(sysctl_post),
        t("perf-netbuf", "Larger network buffers, TCP Fast Open and MTU probing", "16 MB socket buffer ceilings, Fast Open for client and server, and MTU probing: better throughput on fast links and fewer stalls behind broken routers.", "Network")
            .tags(&["perf", "max"])
            .file("/etc/sysctl.d/99-optimaxer-netbuf.conf", "net.core.rmem_max = 16777216\nnet.core.wmem_max = 16777216\nnet.core.netdev_max_backlog = 16384\nnet.ipv4.tcp_fastopen = 3\nnet.ipv4.tcp_mtu_probing = 1\nnet.ipv4.tcp_slow_start_after_idle = 0\n")
            .post(sysctl_post),
        t("perf-gamemode", "GameMode (on-demand game performance)", "Installs gamemode: games started with `gamemoderun %command%` get the performance CPU governor and higher priority only while running.", "Gaming")
            .tags(&["perf", "max"]).pkgs(&["gamemode"]),
        t("perf-irqbalance", "irqbalance (spread hardware interrupts across cores)", "Installs and enables irqbalance: smoother network and disk throughput on multi-core CPUs.", "Performance")
            .tags(&["max"]).pkgs(&["irqbalance"]).enable(&["irqbalance.service"]),
        t("perf-networkd-wait", "Faster boot: mask systemd-networkd-wait-online", "Stops boot waiting on systemd-networkd-wait-online; harmless on NetworkManager systems.", "Performance")
            .risk("moderate").tags(&["max"]).disable(&["systemd-networkd-wait-online.service"]).needs(&["/usr/lib/systemd/system/systemd-networkd-wait-online.service"]),
        // ---- package manager
        t("pm-pacman", "pacman: parallel downloads and colour", "ParallelDownloads = 5 and Color in /etc/pacman.conf: much faster updates, readable output.", "Package manager")
            .tags(&["safe", "perf", "max"]).needs(&["/etc/pacman.conf"]).pm(Pm::Pacman)
            .edit(LineEdit::new("/etc/pacman.conf", "ParallelDownloads", Some("5"), Some("options"), " = "))
            .edit(LineEdit::new("/etc/pacman.conf", "Color", None, Some("options"), " = ")),
        t("pm-paccache", "Weekly package cache cleaning", "paccache.timer keeps the last 3 versions of each package in /var/cache/pacman/pkg. Installs pacman-contrib.", "Package manager")
            .tags(&["safe", "perf", "max"]).pkgs(&["pacman-contrib"]).enable(&["paccache.timer"]).pm(Pm::Pacman),
        t("pm-dnf", "dnf: parallel downloads and fastest mirror", "max_parallel_downloads=10 and fastestmirror=True in /etc/dnf/dnf.conf: much faster updates and installs.", "Package manager")
            .tags(&["safe", "perf", "max"]).needs(&["/etc/dnf/dnf.conf"]).pm(Pm::Dnf)
            .edit(LineEdit::new("/etc/dnf/dnf.conf", "max_parallel_downloads", Some("10"), Some("main"), "="))
            .edit(LineEdit::new("/etc/dnf/dnf.conf", "fastestmirror", Some("True"), Some("main"), "=")),
        // ---- network
        t("net-bbr", "TCP BBR congestion control", "Better throughput and latency on lossy links (Wi-Fi, long distance).", "Network").tags(&["safe", "perf", "max"])
            .file("/etc/modules-load.d/optimaxer-bbr.conf", "tcp_bbr\n")
            .file("/etc/sysctl.d/99-optimaxer-net.conf", "net.core.default_qdisc = fq\nnet.ipv4.tcp_congestion_control = bbr\n")
            .post(&[&["modprobe", "tcp_bbr"], SYSCTL]),
        // ---- security / privacy
        t("sec-sysctl", "Kernel and network hardening", "Hides kernel pointers and dmesg from users, enables reverse-path filtering, ignores ICMP redirects and broadcast pings.", "Security")
            .tags(&["safe", "privacy", "max"])
            .file("/etc/sysctl.d/99-optimaxer-harden.conf",
                "kernel.dmesg_restrict = 1\nkernel.kptr_restrict = 1\nnet.ipv4.conf.all.rp_filter = 1\nnet.ipv4.conf.default.rp_filter = 1\n\
                 net.ipv4.icmp_echo_ignore_broadcasts = 1\nnet.ipv4.conf.all.accept_redirects = 0\nnet.ipv4.conf.default.accept_redirects = 0\n\
                 net.ipv6.conf.all.accept_redirects = 0\nnet.ipv4.conf.all.send_redirects = 0\n")
            .post(sysctl_post),
        t("sec-ufw", "Firewall (ufw): deny incoming, allow outgoing", "Installs and enables ufw. If SSH is running its port is opened first. Blocks incoming connections such as KDE Connect, Samba or game servers until you allow them.", "Security")
            .risk("moderate").tags(&["max"]).pkgs(&["ufw"]).enable(&["ufw.service"]).pm(Pm::Pacman) // Fedora already runs firewalld
            .cmds(&[&["ufw", "default", "deny", "incoming"], &["ufw", "default", "allow", "outgoing"], &["sh", "-c", "systemctl is-active --quiet sshd && ufw allow ssh || true"]])
            .post(&[&["ufw", "--force", "enable"]]).undo_post(&[&["ufw", "--force", "disable"]])
            .test_cmd(&["sh", "-c", "ufw status | grep -q 'Status: active'"]),
        t("priv-avahi", "Disable mDNS/Avahi network discovery", "Stops the machine announcing itself on the LAN. Network printer and Chromecast auto-discovery stops working.", "Privacy")
            .risk("moderate").tags(&["privacy", "max"]).disable(&["avahi-daemon.service", "avahi-daemon.socket"]).needs(&["/usr/lib/systemd/system/avahi-daemon.service"]),
        // ---- browser debloat (managed policies: they apply to every profile and survive browser updates)
        t("browser-firefox", "Firefox: telemetry, studies, Pocket, sponsored content, AI chat",
            &format!("Turns off telemetry, Shield studies, Pocket, sponsored tiles and suggestions, first-run pages, the AI chatbot and link previews; enables strict tracking protection. Also locks the new-tab weather, widgets, stories and sponsored tiles off through an autoconfig file in /usr/lib/firefox. Close every Firefox window and reopen it; about:policies should list the policies.{POLICY_NOTE}"),
            "Browsers").tags(&["privacy", "max"])
            .file("/usr/lib/firefox/distribution/policies.json", policy(&fx, true))
            .file("/etc/firefox/policies/policies.json", policy(&fx, true))
            .file("/usr/lib/firefox/defaults/pref/optimaxer-autoconfig.js", FF_AUTOCONFIG)
            .file("/usr/lib/firefox/optimaxer.cfg", ff_cfg())
            .needs(&["firefox"]),
        t("browser-chromium", "Chromium: reporting, ads API, prediction, AI features",
            &format!("Turns off usage reporting, Privacy Sandbox ad APIs, the spelling web service, search suggestions, network prediction and generative-AI features; WebRTC no longer leaks the local IP.{POLICY_NOTE}"),
            "Browsers").tags(&["privacy", "max"]).file("/etc/chromium/policies/managed/optimaxer.json", policy(&chromium, false)).needs(&["chromium"]),
        t("browser-chrome", "Google Chrome: reporting, ads API, prediction, AI features",
            &format!("Turns off usage reporting, Privacy Sandbox ad APIs, the spelling web service, search suggestions, network prediction, background mode and Gemini/generative-AI features.{POLICY_NOTE}"),
            "Browsers").tags(&["privacy", "max"]).file("/etc/opt/chrome/policies/managed/optimaxer.json", policy(&chromium, false)).needs(&["google-chrome-stable"]),
        t("browser-brave", "Brave: Rewards, Wallet, VPN, AI chat, News, Talk, telemetry",
            &format!("Turns off Brave Rewards, Wallet, VPN, Leo AI chat, News, Talk, P3A and the stats ping, plus the shared Chromium privacy policies.{POLICY_NOTE}"),
            "Browsers").tags(&["privacy", "max"]).file("/etc/brave/policies/managed/optimaxer.json", policy(&brave_policy(), false)).needs(&["brave"]),
        t("browser-edge", "Microsoft Edge: diagnostics, shopping, sidebar, Copilot, rewards",
            &format!("Turns off diagnostic data, the shopping assistant, sidebar, Copilot icon, recommendations, startup boost, workspaces and Rewards, plus the shared Chromium privacy policies.{POLICY_NOTE}"),
            "Browsers").tags(&["privacy", "max"]).file("/etc/opt/edge/policies/managed/optimaxer.json", policy(&edge_policy(), false)).needs(&["microsoft-edge-stable"]),
    ]
}

pub fn tweak_by_id(id: &str) -> Option<Tweak> {
    tweaks().into_iter().find(|t| t.id == id)
}

pub fn preset_ids(env: &Env, preset: &str) -> Vec<String> {
    tweaks().into_iter().filter(|t| t.tags.iter().any(|x| x == preset) && t.available(env)).map(|t| t.id).collect()
}

// ------------------------------------------------------------------------------------------------ apps

#[derive(Clone, Debug, PartialEq)]
pub struct App {
    pub cat: String,
    pub name: String,
    /// 'P' official repo, 'A' AUR, 'F' Flatpak
    pub src: char,
    pub pkg: String,
}

pub fn load_apps(pm: Pm) -> Vec<App> {
    let mut apps = vec![];
    for line in APPS_RAW.trim().lines() {
        let f: Vec<&str> = line.split('|').collect();
        if f.len() != 4 {
            continue;
        }
        let (mut src, mut pkg) = (f[2].chars().next().unwrap(), f[3].to_string());
        if pm == Pm::Dnf {
            let Some((_, s2, p2)) = FEDORA_MAP.iter().find(|(k, _, _)| *k == pkg) else { continue };
            src = *s2;
            pkg = p2.to_string();
        }
        apps.push(App { cat: f[0].into(), name: f[1].into(), src, pkg });
    }
    apps
}

pub fn aur_helper(env: &Env) -> Option<&'static str> {
    ["yay", "paru"].into_iter().find(|h| env.have(h))
}

pub fn app_installed(env: &Env, app: &App) -> bool {
    if app.src == 'F' { flatpak_installed(env, &app.pkg) } else { pkg_installed(env, &app.pkg) }
}

/// Fedora ships Flatpak without the Flathub remote; add it (system-wide) when it is missing.
fn ensure_flathub(env: &Env) {
    let (rc, out) = env.run(&["flatpak", "remotes"]);
    if rc == 0 && out.contains("flathub") {
        return;
    }
    env.info("  adding the Flathub remote");
    env.run_w(&["flatpak", "remote-add", "--if-not-exists", "flathub", "https://dl.flathub.org/repo/flathub.flatpakrepo"]);
}

pub fn install_app(env: &Env, app: &App) -> bool {
    use crate::sys::{failure_tail, pkg_install};
    env.info(&format!("Install {} ({}:{})", app.name, app.src, app.pkg));
    if app_installed(env, app) {
        env.info("  already installed");
        return true;
    }
    let (rc, out) = match app.src {
        'P' => pkg_install(env, std::slice::from_ref(&app.pkg)),
        'A' => {
            let Some(helper) = aur_helper(env) else {
                env.warn("  needs an AUR helper: install 'yay' from the Install tab first");
                return false;
            };
            env.run_user_w(&[helper, "-S", "--needed", "--noconfirm", &app.pkg])
        }
        _ => {
            if !env.have("flatpak") {
                env.warn("  needs flatpak: install 'Flatpak' from the Utilities group first");
                return false;
            }
            ensure_flathub(env);
            env.run_w(&["flatpak", "install", "-y", "--noninteractive", "flathub", &app.pkg])
        }
    };
    if env.dry_run {
        return true;
    }
    let ok = rc == 0 && app_installed(env, app);
    if ok { env.ok("  installed and verified") } else { env.warn(&format!("  FAILED: {}", failure_tail(rc, &out))) }
    ok
}

pub fn remove_app(env: &Env, app: &App) -> bool {
    use crate::env::tail;
    use crate::sys::pkg_remove;
    env.info(&format!("Remove {}", app.name));
    if !app_installed(env, app) {
        env.info("  not installed");
        return true;
    }
    let (rc, out) = if app.src == 'F' {
        env.run_w(&["flatpak", "uninstall", "-y", "--noninteractive", &app.pkg])
    } else {
        pkg_remove(env, std::slice::from_ref(&app.pkg))
    };
    if env.dry_run {
        return true;
    }
    let ok = rc == 0 && !app_installed(env, app);
    if ok { env.ok("  removed and verified") } else { env.warn(&format!("  FAILED: {}", tail(&out, 160))) }
    ok
}

// ------------------------------------------------------------------------------------------------ debloat

pub struct DebloatRow {
    pub pkg: String,
    pub label: String,
    pub size: u64,
}

pub fn installed_debloat(env: &Env) -> Vec<DebloatRow> {
    let list = if env.pm == Pm::Dnf { DEBLOAT_FEDORA } else { DEBLOAT_ARCH };
    list.iter()
        .filter(|(pkg, _)| pkg_installed(env, pkg))
        .map(|(pkg, label)| DebloatRow { pkg: pkg.to_string(), label: label.to_string(), size: pkg_size(env, pkg) })
        .collect()
}

// ------------------------------------------------------------------------------------------------ cleaner

#[derive(Clone, Debug)]
pub struct CleanTarget {
    pub id: &'static str,
    pub name: &'static str,
    pub risk: &'static str,
    pub paths: Vec<&'static str>,
    pub home: Vec<&'static str>,
    pub special: Option<&'static str>,
}

pub fn clean_targets(pm: Pm) -> Vec<CleanTarget> {
    let t = |id, name, risk, paths: &[&'static str], home: &[&'static str], special| CleanTarget { id, name, risk, paths: paths.to_vec(), home: home.to_vec(), special };
    let all = vec![
        t("pacman-cache", "pacman package cache (keep 2 newest versions)", "safe", &["/var/cache/pacman/pkg"], &[], None),
        t("orphans", "Orphaned packages (installed as dependencies, no longer needed)", "moderate", &[], &[], Some("orphans")),
        t("journal", "systemd journal (keep 100 MB)", "safe", &["/var/log/journal"], &[], None),
        t("coredumps", "Core dumps", "safe", &["/var/lib/systemd/coredump"], &[], None),
        t("yay", "AUR helper build cache (yay/paru)", "safe", &[], &[".cache/yay", ".cache/paru"], None),
        t("thumbs", "Thumbnail cache", "safe", &[], &[".cache/thumbnails"], None),
        t("trash", "Trash", "moderate", &[], &[".local/share/Trash/files", ".local/share/Trash/info"], None),
        t("firefox", "Firefox cache", "safe", &[], &[".cache/mozilla/firefox"], None),
        t("chromium", "Chromium / Chrome / Brave / Vivaldi cache", "safe", &[], &[".cache/chromium", ".cache/google-chrome", ".cache/BraveSoftware", ".cache/vivaldi"], None),
        t("pip", "pip cache", "safe", &[], &[".cache/pip"], None),
        t("npm", "npm cache", "safe", &[], &[".npm/_cacache"], None),
        t("go", "Go build cache", "safe", &[], &[".cache/go-build"], None),
        t("cargo", "Rust cargo download cache", "moderate", &[], &[".cargo/registry/cache"], None),
        t("flatpak-unused", "Unused Flatpak runtimes", "safe", &[], &[], Some("flatpak")),
        t("dnf-cache", "dnf package and metadata cache", "safe", &["/var/cache/libdnf5", "/var/cache/dnf"], &[], None),
    ];
    let skip: &[&str] = match pm {
        Pm::Pacman => &["dnf-cache"],
        Pm::Dnf => &["pacman-cache", "yay"],
    };
    all.into_iter().filter(|c| !skip.contains(&c.id)).collect()
}

impl CleanTarget {
    pub fn target_paths(&self, env: &Env) -> Vec<String> {
        let home = env.real_home();
        self.paths.iter().map(|p| p.to_string()).chain(self.home.iter().map(|h| format!("{}/{}", home.trim_end_matches('/'), h))).collect()
    }
}

/// Bytes the target would free; -1 when the size is only known on demand.
pub fn target_size(env: &Env, t: &CleanTarget) -> i64 {
    match t.special {
        Some("orphans") => orphans(env).iter().map(|p| pkg_size(env, p) as i64).sum(),
        Some(_) => -1,
        None => t.target_paths(env).iter().map(|p| env.dir_size(p) as i64).sum(),
    }
}

/// A symlink at the target, or at any directory between it and `stop` (exclusive), could redirect deletion elsewhere.
fn redirected(path: &str, stop: &str) -> bool {
    let mut cur = std::path::PathBuf::from(path);
    let stop = std::path::Path::new(stop);
    loop {
        if cur == stop || cur.parent().is_none() {
            return false;
        }
        match std::fs::symlink_metadata(&cur) {
            Ok(md) if md.file_type().is_symlink() => return true,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return true, // do not delete through a directory whose identity cannot be checked
        }
        cur = cur.parent().unwrap().to_path_buf();
    }
}

/// Clean one target; returns bytes freed (best effort).
pub fn clean_target(env: &Env, t: &CleanTarget) -> u64 {
    use crate::env::tail;
    use crate::sys::{clean_cache, pkg_remove};
    let before = target_size(env, t).max(0);
    env.info(&format!("Clean: {}", t.name));
    match t.id {
        "pacman-cache" | "dnf-cache" => clean_cache(env),
        "orphans" => {
            let pk = orphans(env);
            if !pk.is_empty() {
                let (rc, out) = pkg_remove(env, &pk);
                if rc != 0 {
                    env.warn(&format!("Package removal failed: {}", tail(&out, 160)));
                }
            }
        }
        "journal" => {
            env.change_s(&["journalctl", "--vacuum-size=100M"]);
        }
        "flatpak-unused" => {
            if env.have("flatpak") {
                env.change_s(&["flatpak", "uninstall", "--unused", "-y"]);
            }
        }
        _ => {
            let home = env.p(&env.real_home());
            for p in t.target_paths(env) {
                let full = env.p(&p);
                // Home targets may only be reached through ordinary directories below the home folder; system targets must not be links themselves.
                let stop = if t.home.is_empty() { std::path::Path::new(&full).parent().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default() } else { home.clone() };
                if redirected(&full, &stop) {
                    env.warn(&format!("Skipping redirected cleanup target: {p}"));
                    continue;
                }
                let Ok(rd) = std::fs::read_dir(&full) else { continue };
                for entry in rd.flatten() {
                    let path = entry.path();
                    if env.dry_run {
                        env.log(&format!("[dry-run] remove {}", path.display()), "DRY");
                        continue;
                    }
                    let is_dir = entry.file_type().map(|f| f.is_dir()).unwrap_or(false);
                    let r = if is_dir { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
                    if let Err(e) = r {
                        env.warn(&format!("Could not remove {}: {e}", path.display()));
                    }
                }
            }
        }
    }
    let after = if env.dry_run { before } else { target_size(env, t).max(0) };
    (before - after).max(0) as u64
}

pub fn size_label(n: i64) -> String {
    if n < 0 { "on demand".into() } else { crate::env::fmt_size(n as f64) }
}
