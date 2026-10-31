//! Network: adapters, the DNS servers in use, DNS provider switching through NetworkManager, and a lookup timing test.

use super::*;
use crate::linux::fmt_size;
use crate::linux::network::PROVIDERS;

const LOOKUP_HOSTS: [&str; 3] = ["example.com", "cloudflare.com", "wikipedia.org"];

impl OptiUi {
    pub(super) fn network_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        kit::page_header(
            ui,
            "Network",
            "Adapters and DNS as the system reports them. Changing DNS goes through NetworkManager, needs no administrator password, and the previous settings are saved.",
        );
        if kit::ghost(ui, "Refresh", self.busy.is_none()).clicked() {
            self.refresh_network(ctx);
        }
        let Some(data) = &self.network else {
            kit::skeleton(ui, 4);
            return;
        };
        kit::group_title(ui, "Adapters");
        if data.adapters.is_empty() {
            ui.label(
                RichText::new("No network adapters found.")
                    .size(T_SMALL)
                    .color(t.subtle),
            );
        }
        for a in &data.adapters {
            kit::row(
                ui,
                ("linux-nic", &a.name),
                data.default_interface.as_deref() == Some(a.name.as_str()),
                |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(&a.name)
                                .font(FontId::new(T_BODY, semibold()))
                                .color(t.text),
                        );
                        Self::badge(
                            ui,
                            if a.up { "Up" } else { "Down" },
                            if a.up { Tone::Good } else { Tone::Neutral },
                        );
                        if a.wireless {
                            Self::badge(ui, "Wi-Fi", Tone::Neutral);
                        }
                        if data.default_interface.as_deref() == Some(a.name.as_str()) {
                            Self::badge(ui, "Default route", Tone::Brand);
                        }
                        let mut facts = Vec::new();
                        if let Some(speed) = a.speed_mbps {
                            facts.push(format!("{speed} Mb/s"));
                        }
                        if let Some(mtu) = a.mtu {
                            facts.push(format!("MTU {mtu}"));
                        }
                        if let Some(mac) = &a.mac {
                            facts.push(mac.clone());
                        }
                        if let (Some(rx), Some(tx)) = (a.rx_bytes, a.tx_bytes) {
                            facts.push(format!("received {}, sent {}", fmt_size(rx), fmt_size(tx)));
                        }
                        ui.label(
                            RichText::new(facts.join("  ·  "))
                                .size(T_SMALL)
                                .color(t.muted),
                        );
                    });
                },
            );
        }

        kit::group_title(ui, "DNS");
        let servers = if data.dns.is_empty() {
            "none reported".to_string()
        } else {
            data.dns.join(", ")
        };
        ui.label(
            RichText::new(format!("In use now: {servers}"))
                .size(T_SMALL)
                .color(t.muted),
        );
        let mut chosen: Option<usize> = None;
        let mut restore = false;
        match &data.connections {
            Err(error) => {
                ui.add(egui::Label::new(RichText::new(error).size(T_SMALL).color(t.subtle)).wrap());
            }
            Ok(connections) => {
                let names: Vec<&str> = connections.iter().map(|c| c.name.as_str()).collect();
                ui.label(
                    RichText::new(format!(
                        "Active connections: {}",
                        if names.is_empty() {
                            "none".into()
                        } else {
                            names.join(", ")
                        }
                    ))
                    .size(T_CAPTION)
                    .color(t.subtle),
                );
                ui.add_space(S1);
                ui.horizontal_wrapped(|ui| {
                    for (i, p) in PROVIDERS.iter().enumerate() {
                        ui.push_id(p.key, |ui| {
                            if kit::secondary(
                                ui,
                                p.name,
                                self.busy.is_none() && !connections.is_empty(),
                            )
                            .clicked()
                            {
                                chosen = Some(i);
                            }
                        });
                    }
                });
                if !self.state.dns.is_empty() {
                    ui.add_space(S1);
                    ui.horizontal(|ui| {
                        Self::badge(ui, "Changed by Optimaxer", Tone::Neutral);
                        if kit::ghost(ui, "Restore previous DNS", self.busy.is_none()).clicked() {
                            restore = true;
                        }
                    });
                }
            }
        }

        kit::group_title(ui, "Lookup speed");
        let mut run_lookup = false;
        ui.horizontal(|ui| {
            if kit::secondary(
                ui,
                if self.lookup_running {
                    "Testing"
                } else {
                    "Test DNS lookups"
                },
                !self.lookup_running,
            )
            .clicked()
            {
                run_lookup = true;
            }
            ui.label(
                RichText::new(
                    "Times the system resolver for three names. Answers may come from a cache.",
                )
                .size(T_CAPTION)
                .color(t.subtle),
            );
        });
        if let Some(results) = &self.lookups {
            for (host, result) in results {
                let text = match result {
                    Ok(d) => format!("{host}: {} ms", d.as_millis()),
                    Err(e) => e.to_string(),
                };
                ui.label(RichText::new(text).size(T_SMALL).color(t.muted));
            }
        }

        if run_lookup {
            self.lookup_running = true;
            self.spawn(ctx, |_| {
                Msg::Lookup(
                    LOOKUP_HOSTS
                        .iter()
                        .map(|h| (h.to_string(), network::lookup_time(h)))
                        .collect(),
                )
            });
        }
        if let Some(i) = chosen {
            let provider = &PROVIDERS[i];
            let saved: Vec<_> = self.state.dns.values().cloned().collect();
            match network::prepare_dns(self.sys.as_ref(), provider, &saved) {
                Ok(change) => {
                    let mut state = self.state.clone();
                    let mut lines = Vec::new();
                    for snap in &change.snapshots {
                        lines.push(Line::new(format!(
                            "{}: currently IPv4 DNS “{}”, IPv6 DNS “{}”",
                            snap.name, snap.ipv4_dns, snap.ipv6_dns
                        )));
                        state.dns.insert(snap.name.clone(), snap.clone());
                    }
                    let pending = self.plan_pending(
                        format!("Use {} DNS", provider.name),
                        if provider.v4.is_empty() {
                            "NetworkManager goes back to the DNS servers your router provides."
                                .to_string()
                        } else {
                            format!(
                                "Active connections will ask {} instead of your router.",
                                provider.v4.join(" and ")
                            )
                        },
                        lines,
                        change.plan,
                        "Change DNS",
                        Followup::Network,
                        Some(state),
                        None,
                    );
                    self.ask(pending);
                }
                Err(error) => self.toast(error),
            }
        }
        if restore {
            let snaps: Vec<_> = self.state.dns.values().cloned().collect();
            match network::prepare_dns_restore(self.sys.as_ref(), &snaps) {
                Ok(plan) => {
                    let mut state = self.state.clone();
                    state.dns.clear();
                    let lines = snaps
                        .iter()
                        .map(|s| {
                            Line::new(format!(
                                "{}: IPv4 “{}”, IPv6 “{}”",
                                s.name, s.ipv4_dns, s.ipv6_dns
                            ))
                        })
                        .collect();
                    let pending = self.plan_pending(
                        "Restore previous DNS",
                        "The DNS settings saved before the change are written back.",
                        lines,
                        plan,
                        "Restore",
                        Followup::Network,
                        None,
                        Some(state),
                    );
                    self.ask(pending);
                }
                Err(error) => self.toast(error),
            }
        }
    }
}
