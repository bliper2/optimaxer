//! Network page of the Optimaxer section.

use super::*;

impl OptiUi {
    pub(super) fn network_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        kit::page_header(
            ui,
            "Network",
            "Switch DNS, compare resolver responses, flush the cache, or reset the network stack.",
        );
        let idle = self.busy.is_none();
        ui.horizontal_wrapped(|ui| {
            if kit::secondary(ui, "Show current DNS", idle).clicked() {
                self.spawn(ctx, "Reading DNS settings", |w| {
                    w.send(Msg::Adapters(network::active_adapters()));
                    network::show_dns(&WinSystem, &mut |l| w.log(l));
                });
            }
            if kit::secondary(ui, "Flush DNS cache", idle).clicked() {
                self.spawn_recorded(ctx, "Flush DNS", |w| {
                    let result = WinSystem.run("ipconfig.exe", &["/flushdns"]);
                    if result.code == 0 {
                        w.log("DNS cache flushed.");
                    } else {
                        w.log(&format!("WARN DNS cache flush failed: {}", result.text.trim()));
                    }
                });
            }
            if kit::secondary(ui, "Benchmark servers", idle).clicked() {
                let data = self.data.clone();
                self.spawn(ctx, "Benchmarking DNS servers", move |w| {
                    let mut rows = Vec::new();
                    for p in data.dns.iter().filter(|p| !p.v4.is_empty()) {
                        let result = network::benchmark_dns(&p.v4[0], 5);
                        let timing = result.average_ms.map(|ms| format!("{ms:.1} ms average")).unwrap_or_else(|| "no valid DNS answers".into());
                        let jitter = result.jitter_ms.map(|ms| format!("{ms:.1} ms jitter")).unwrap_or_else(|| "jitter unavailable".into());
                        w.log(&format!("  {}: {timing}, {jitter}, {}/{} answers", p.name, result.answers, result.attempts));
                        rows.push((p.name.clone(), result));
                    }
                    if let Some((name, ms)) = rows.iter().filter_map(|(name, result)| result.average_ms.map(|ms| (name, ms))).min_by(|a, b| a.1.total_cmp(&b.1)) {
                        w.log(&format!("Lowest measured DNS query average: {name} ({ms:.1} ms). This does not predict ping to game servers."));
                    }
                    w.send(Msg::Bench(rows));
                });
            }
            if kit::ghost(ui, "Reset network stack", idle).clicked() && self.need_admin() {
                self.pending = Some(Pending { title: "Reset network stack".into(), intro: "Reset Winsock and the IP stack? You may briefly lose connection and need to restart Windows.".into(), lines: vec![], action: Action::ResetNetwork, restore_option: false, ok_label: "Reset network stack".into() });
            }
            if network::dns_backup_available() {
                ui.label(RichText::new("Original DNS settings saved").size(T_CAPTION).color(t.warn));
                if kit::secondary(ui, "Restore original DNS", idle).clicked() && self.need_admin() {
                    self.pending = Some(Pending {
                        title: "Restore original DNS settings".into(),
                        intro: "Return adapters and DNS-over-HTTPS entries to the values saved before Optimaxer changed them. Settings changed outside Optimaxer are left untouched; incomplete restores keep the backup for retry.".into(),
                        lines: vec![],
                        action: Action::RestoreDns,
                        restore_option: false,
                        ok_label: "Restore DNS".into(),
                    });
                }
            }
        });
        ui.add_space(S1);
        ui.label(RichText::new("Sends five UDP A-record lookups for example.com directly to each provider's first IPv4 server; DNS settings are not changed. Jitter is mean absolute change between consecutive successful response times. Failed probes are not IP packet-loss measurements.").size(T_CAPTION).color(t.subtle));
        if !self.bench.is_empty() {
            ui.label(
                RichText::new("Resolver response time does not predict latency to a game server.")
                    .size(T_CAPTION)
                    .color(t.subtle),
            );
        }
        if !self.adapters.is_empty() {
            ui.label(
                RichText::new(format!("Active adapters: {}", self.adapters.join(", ")))
                    .size(T_CAPTION)
                    .color(t.subtle),
            );
        }
        kit::group_title(ui, "DNS provider");
        let providers = self.data.dns.clone();
        ui.horizontal_wrapped(|ui| {
            for p in providers {
                let ms = self.bench.iter().find(|b| b.0 == p.name).map(|(_, result)| {
                    let average = result.average_ms.map(|m| format!("{m:.1} ms avg")).unwrap_or_else(|| "No DNS answer".into());
                    let jitter = result.jitter_ms.map(|m| format!(" · {m:.1} ms jitter")).unwrap_or_default();
                    format!("{average}{jitter} · {}/{}", result.answers, result.attempts)
                });
                if kit::tile(ui, vec2(252.0, 84.0), &p.name, &p.desc, ms.as_deref(), idle, false).clicked() && self.need_admin() {
                    self.pending = Some(Pending {
                        title: format!("Use {} for DNS", p.name),
                        intro: format!("Set DNS to {} on all active network adapters? Optimaxer saves the current IPv4, IPv6 and affected DNS-over-HTTPS settings first. Changes stop if that backup cannot be read or written.", p.name),
                        lines: p.v4.iter().chain(p.v6.iter()).map(|s| Line { text: s.clone(), warn: false, head: false }).collect(),
                        action: Action::SetDns(p.clone()),
                        restore_option: false,
                        ok_label: format!("Use {}", p.name),
                    });
                }
            }
        });
    }
}
