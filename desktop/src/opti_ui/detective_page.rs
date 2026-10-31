//! Bottleneck Detective page: a guided 30-second measurement (or the last hour of Black Box history), analysed into
//! observed findings and clearly separated hypotheses.

use super::*;
use crate::opti::blackbox::{self, Sample};
use crate::opti::detective::{self, Certainty, Destination, Evidence, ProcessLoad, Report};
use crate::opti::sysmon::Sampler;

const CHECK_SECONDS: u64 = 30;

struct Running {
    cancel: Arc<AtomicBool>,
    seconds_done: u64,
}

#[derive(Default)]
pub(super) struct State {
    running: Option<Running>,
    result: Option<(Evidence, Report)>,
    status: Option<String>,
}

impl State {
    pub(super) fn progress(&mut self, seconds: u64) {
        if let Some(running) = self.running.as_mut() {
            running.seconds_done = seconds;
        }
    }

    pub(super) fn finish(&mut self, outcome: Result<Evidence, String>) {
        self.running = None;
        match outcome {
            Ok(evidence) => {
                let report = detective::analyze(&evidence);
                self.status = None;
                self.result = Some((evidence, report));
            }
            Err(error) => self.status = Some(error),
        }
    }
}

impl OptiUi {
    pub(super) fn detective_page(&mut self, ui: &mut Ui, ctx: &egui::Context, settings: &Settings) {
        let t = tokens(ctx);
        kit::page_header(
            ui,
            "Bottleneck Detective",
            "Measures what is working hardest and explains what the readings do and do not show.",
        );
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::Label::new(
                    RichText::new(format!(
                        "Start whatever feels slow (a game, a build, a heavy page), then press Start. Optimaxer takes {CHECK_SECONDS} one-second readings of processor, memory, disk, network and graphics, and notes which programs used the processor. It changes nothing and sends nothing."
                    ))
                    .size(T_SMALL)
                    .color(t.muted),
                )
                .wrap(),
            );
            ui.add_space(S2);
            ui.horizontal_wrapped(|ui| {
                match self.detective.running.as_ref() {
                    Some(running) => {
                        let fraction = running.seconds_done as f32 / CHECK_SECONDS as f32;
                        ui.add(
                            egui::ProgressBar::new(fraction)
                                .desired_width(220.0)
                                .text(format!("{} / {CHECK_SECONDS} s", running.seconds_done)),
                        );
                        if kit::secondary(ui, "Cancel", true).clicked() {
                            running.cancel.store(true, Ordering::Relaxed);
                        }
                    }
                    None => {
                        if kit::primary(ui, "Start 30-second check", true).clicked() {
                            self.start_detective_check(ctx);
                        }
                        if kit::secondary(ui, "Analyse Black Box (last hour)", true)
                            .on_hover_text("Uses readings already recorded; needs Black Box recording to have been on")
                            .clicked()
                        {
                            self.analyse_blackbox_history(settings);
                        }
                    }
                }
            });
            if let Some(status) = &self.detective.status {
                ui.label(RichText::new(status).size(T_SMALL).color(t.warn));
            }
        });
        ui.add_space(S2);
        let Some((evidence, report)) = &self.detective.result else {
            return;
        };
        let mut go_to = None;
        ui.label(
            RichText::new(format!(
                "{} · {} readings",
                evidence.source, report.readings
            ))
            .size(T_CAPTION)
            .color(t.subtle),
        );
        ui.add_space(S1);
        if report.too_short {
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(format!(
                        "Fewer than {} readings were available, which is not enough to say anything reliable. Run the 30-second check, or leave Black Box recording on for longer.",
                        detective::MIN_READINGS
                    ))
                    .size(T_SMALL)
                    .color(t.muted),
                );
            });
        }
        if !report.too_short && report.findings.is_empty() {
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("Nothing in these readings points to a bottleneck")
                        .font(FontId::new(T_HEADING, semibold()))
                        .color(t.text),
                );
                ui.label(
                    RichText::new("That covers only what was measured. If it still felt slow, repeat the check while the slowdown is happening.")
                        .size(T_SMALL)
                        .color(t.muted),
                );
            });
            ui.add_space(S2);
        }
        for finding in &report.findings {
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    match finding.certainty {
                        Certainty::Observed => kit::chip(ui, "Observed", Tone::Brand),
                        Certainty::Hypothesis => kit::chip(ui, "Hypothesis", Tone::Warn),
                    }
                    kit::chip(ui, finding.area.label(), Tone::Neutral);
                });
                ui.add(
                    egui::Label::new(
                        RichText::new(&finding.headline)
                            .font(FontId::new(T_BODY, semibold()))
                            .color(t.text),
                    )
                    .wrap(),
                );
                for line in &finding.evidence {
                    ui.add(
                        egui::Label::new(RichText::new(line).size(T_SMALL).color(t.muted)).wrap(),
                    );
                }
                if finding.certainty == Certainty::Hypothesis {
                    ui.label(
                        RichText::new("The readings fit this explanation but do not prove it.")
                            .size(T_CAPTION)
                            .color(t.subtle),
                    );
                }
                for (step, destination) in &finding.next_steps {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(step).size(T_SMALL).color(t.text));
                        if let Some(destination) = destination
                            && kit::secondary(ui, destination_label(*destination), true).clicked()
                        {
                            go_to = Some(*destination);
                        }
                    });
                }
            });
            ui.add_space(S2);
        }
        if !report.unremarkable.is_empty() {
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("Measured and unremarkable")
                        .font(FontId::new(T_HEADING, semibold()))
                        .color(t.text),
                );
                for (area, detail) in &report.unremarkable {
                    ui.label(
                        RichText::new(format!("{}: {detail}", area.label()))
                            .size(T_SMALL)
                            .color(t.muted),
                    );
                }
            });
            ui.add_space(S2);
        }
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("Not assessed")
                    .font(FontId::new(T_HEADING, semibold()))
                    .color(t.text),
            );
            for (name, reason) in &report.not_assessed {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("{name}: {reason}"))
                            .size(T_SMALL)
                            .color(t.muted),
                    )
                    .wrap(),
                );
            }
            ui.add_space(S1);
            ui.label(
                RichText::new("These are observations from a short window, not a diagnosis, and there is no score. Change one thing at a time and measure again before trusting a fix.")
                    .size(T_CAPTION)
                    .color(t.subtle),
            );
        });
        ui.add_space(S3);
        if let Some(destination) = go_to {
            self.set_page(ctx, destination_page(destination));
        }
    }

    fn start_detective_check(&mut self, ctx: &egui::Context) {
        let cancel = Arc::new(AtomicBool::new(false));
        self.detective.running = Some(Running {
            cancel: cancel.clone(),
            seconds_done: 0,
        });
        self.detective.status = None;
        let threads = self
            .info
            .as_ref()
            .map(|info| info.threads)
            .filter(|threads| *threads > 0)
            .or_else(|| {
                std::thread::available_parallelism()
                    .ok()
                    .map(|n| n.get() as u32)
            })
            .unwrap_or(1);
        let worker = self.worker(ctx);
        std::thread::spawn(move || {
            let outcome = live_check(&worker, &cancel, threads);
            worker.send(Msg::DetectiveDone(outcome));
        });
    }

    fn analyse_blackbox_history(&mut self, settings: &Settings) {
        let store = match &self.blackbox {
            Some(recorder) => recorder.snapshot(),
            None => blackbox::load(&blackbox::default_path()).unwrap_or_default(),
        };
        let now = crate::opti::now_secs();
        let samples = store.between(now.saturating_sub(3_600), now);
        self.detective.finish(Ok(Evidence {
            source: "Black Box, last hour".into(),
            samples,
            interval_seconds: super::blackbox_page::blackbox_interval(settings),
            drive_free_fraction: drive_free_fraction(),
            startup_enabled: None,
            busy_processes: Vec::new(),
        }));
    }
}

fn live_check(worker: &Worker, cancel: &AtomicBool, threads: u32) -> Result<Evidence, String> {
    use crate::opti::processes::{self, CpuTimes};
    let (_, baseline) = processes::scan(&CpuTimes::new(), threads)?;
    let mut sampler = Sampler::new();
    let mut samples: Vec<Sample> = Vec::with_capacity(CHECK_SECONDS as usize);
    for second in 1..=CHECK_SECONDS {
        std::thread::sleep(Duration::from_secs(1));
        if cancel.load(Ordering::Relaxed) {
            return Err("The check was cancelled.".into());
        }
        sampler.sample();
        samples.push(Sample::from_snapshot(
            crate::opti::now_secs(),
            &sampler.latest,
        ));
        worker.send(Msg::DetectiveProgress(second));
    }
    let (after, _) = processes::scan(&baseline, threads)?;
    // Several processes of one program (a browser, for example) are added together.
    let mut by_name: HashMap<String, ProcessLoad> = HashMap::new();
    for process in after {
        let Some(cpu) = process.cpu_percent else {
            continue;
        };
        let entry = by_name
            .entry(process.name.to_lowercase())
            .or_insert_with(|| ProcessLoad {
                name: process.name.clone(),
                cpu_percent: 0.0,
                working_set: Some(0),
            });
        entry.cpu_percent += cpu;
        entry.working_set = entry
            .working_set
            .zip(process.working_set)
            .map(|(a, b)| a + b);
    }
    let mut busy: Vec<ProcessLoad> = by_name.into_values().collect();
    busy.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent));
    busy.truncate(8);
    let startup_enabled = Some(
        startup::list(&WinSystem)
            .items
            .iter()
            .filter(|item| item.enabled)
            .count(),
    );
    Ok(Evidence {
        source: format!("Live {CHECK_SECONDS}-second check"),
        samples,
        interval_seconds: 1,
        drive_free_fraction: drive_free_fraction(),
        startup_enabled,
        busy_processes: busy,
    })
}

fn drive_free_fraction() -> Option<f32> {
    crate::opti::sysmon::read_disk()
        .filter(|(_, total)| *total > 0)
        .map(|(used, total)| 1.0 - used as f32 / total as f32)
}

fn destination_label(destination: Destination) -> &'static str {
    match destination {
        Destination::Processes => "Open Processes",
        Destination::Startup => "Open Startup",
        Destination::Cleaner => "Open Cleaner",
    }
}

fn destination_page(destination: Destination) -> Page {
    match destination {
        Destination::Processes => Page::Processes,
        Destination::Startup => Page::Startup,
        Destination::Cleaner => Page::Cleaner,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finishing_with_an_error_keeps_the_previous_result_and_reports_why() {
        let mut state = State::default();
        state.finish(Ok(Evidence::default()));
        assert!(state.result.is_some());
        state.finish(Err("The check was cancelled.".into()));
        assert!(state.result.is_some());
        assert_eq!(state.status.as_deref(), Some("The check was cancelled."));
        assert!(state.running.is_none());
    }

    #[test]
    fn every_destination_opens_a_page() {
        for destination in [
            Destination::Processes,
            Destination::Startup,
            Destination::Cleaner,
        ] {
            assert!(!destination_label(destination).is_empty());
            let _ = destination_page(destination);
        }
        assert_eq!(
            crate::opti::detective::Area::Startup.label(),
            "Startup load"
        );
    }

    #[test]
    #[ignore = "measures this PC for 30 seconds"]
    fn live_check_collects_real_readings_and_reports_progress() {
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = Worker {
            tx,
            ctx: egui::Context::default(),
            cancel: None,
            operation_details: None,
            operation_status: None,
        };
        let evidence = live_check(&worker, &AtomicBool::new(false), 8).expect("check completes");
        assert_eq!(evidence.samples.len(), CHECK_SECONDS as usize);
        assert!(evidence.samples.iter().any(|s| s.cpu.is_some()));
        assert!(evidence.samples.iter().any(|s| s.ram.is_some()));
        assert!(evidence.drive_free_fraction.is_some());
        let progress = rx
            .try_iter()
            .filter(|m| matches!(m, Msg::DetectiveProgress(_)))
            .count();
        assert_eq!(progress, CHECK_SECONDS as usize);
        let report = detective::analyze(&evidence);
        println!(
            "findings: {:#?}
unremarkable: {:?}",
            report.findings, report.unremarkable
        );
    }

    #[test]
    fn a_cancelled_check_returns_an_error_not_partial_evidence() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let worker = Worker {
            tx,
            ctx: egui::Context::default(),
            cancel: None,
            operation_details: None,
            operation_status: None,
        };
        let result = live_check(&worker, &AtomicBool::new(true), 8);
        assert_eq!(result.unwrap_err(), "The check was cancelled.");
    }
}
