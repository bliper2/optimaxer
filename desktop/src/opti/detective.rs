//! Bottleneck Detective: turns a short window of measurements into evidence-backed findings. Every finding is marked
//! either *observed* (the readings show it) or a *hypothesis* (a plausible explanation the readings cannot confirm).
//! There is no score or health percentage, and areas Optimaxer cannot measure are listed as not assessed.

use super::blackbox::Sample;

/// A reading at or above this share counts as saturated.
pub const SATURATED_PERCENT: f32 = 90.0;
/// GPU engines are busy-limited a little below full because the counter is the busiest engine's utilisation.
pub const GPU_BUSY_PERCENT: f32 = 95.0;
/// Fewer readings than this cannot support a conclusion.
pub const MIN_READINGS: usize = 10;
/// Share of readings that must be saturated before the area is reported.
const MIN_SATURATED_SHARE: f32 = 0.20;
const LOW_FREE_SPACE: f32 = 0.10;
const MANY_STARTUP_ITEMS: usize = 15;
const BUSY_PROCESS_PERCENT: f32 = 5.0;

#[derive(Clone, Debug, PartialEq)]
pub struct ProcessLoad {
    pub name: String,
    /// Average share of total CPU capacity over the measured window.
    pub cpu_percent: f32,
    pub working_set: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct Evidence {
    pub source: String,
    pub samples: Vec<Sample>,
    pub interval_seconds: u64,
    pub drive_free_fraction: Option<f32>,
    pub startup_enabled: Option<usize>,
    /// Empty when process activity was not measured for this window.
    pub busy_processes: Vec<ProcessLoad>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Certainty {
    Observed,
    Hypothesis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Area {
    Processor,
    Graphics,
    Memory,
    Storage,
    Startup,
    BackgroundPrograms,
}

impl Area {
    pub fn label(self) -> &'static str {
        match self {
            Area::Processor => "Processor",
            Area::Graphics => "Graphics",
            Area::Memory => "Memory",
            Area::Storage => "Storage",
            Area::Startup => "Startup load",
            Area::BackgroundPrograms => "Background programs",
        }
    }
}

/// A place the user can go to act on a finding. The UI maps these to pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Processes,
    Startup,
    Cleaner,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub area: Area,
    pub certainty: Certainty,
    pub headline: String,
    pub evidence: Vec<String>,
    pub next_steps: Vec<(String, Option<Destination>)>,
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub findings: Vec<Finding>,
    /// Areas that were measured and showed nothing notable, with the numbers behind that.
    pub unremarkable: Vec<(Area, String)>,
    pub not_assessed: Vec<(&'static str, String)>,
    pub readings: usize,
    pub too_short: bool,
}

fn values(samples: &[Sample], pick: fn(&Sample) -> Option<f32>) -> Vec<f32> {
    samples.iter().filter_map(pick).collect()
}

fn average(v: &[f32]) -> f32 {
    v.iter().sum::<f32>() / v.len() as f32
}

fn peak(v: &[f32]) -> f32 {
    v.iter().copied().fold(0.0, f32::max)
}

fn share_at_or_above(v: &[f32], limit: f32) -> f32 {
    v.iter().filter(|x| **x >= limit).count() as f32 / v.len() as f32
}

/// The longest run of consecutive readings at or above the limit, in seconds.
fn longest_run_seconds(samples: &[Sample], pick: fn(&Sample) -> Option<f32>, limit: f32) -> u64 {
    let (mut best, mut start) = (0u64, None::<u64>);
    for s in samples {
        match pick(s) {
            Some(v) if v >= limit => {
                let from = *start.get_or_insert(s.t);
                best = best.max(s.t - from);
            }
            _ => start = None,
        }
    }
    best
}

fn percent(share: f32) -> String {
    format!("{:.0}%", share * 100.0)
}

fn percentile(v: &[f32], p: f32) -> f32 {
    let mut sorted = v.to_vec();
    sorted.sort_by(f32::total_cmp);
    sorted[((sorted.len() - 1) as f32 * p).round() as usize]
}

pub fn analyze(e: &Evidence) -> Report {
    let mut report = Report {
        readings: e.samples.len(),
        ..Report::default()
    };
    report.not_assessed.extend([
        (
            "Thermal throttling",
            "Windows does not expose temperature or throttling counters to Optimaxer on this PC, so heat cannot be ruled in or out.".to_owned(),
        ),
        (
            "Network stability",
            "This check sends no test traffic. The Network page measures DNS response time; it does not measure game-server latency.".to_owned(),
        ),
        (
            "Disk saturation",
            "Only throughput is read, not queue length or latency, so a busy disk cannot be told from a fast one working normally.".to_owned(),
        ),
    ]);
    if e.samples.len() < MIN_READINGS {
        report.too_short = true;
        return report;
    }
    let cpu = values(&e.samples, |s| s.cpu);
    let ram = values(&e.samples, |s| s.ram);
    let gpu = values(&e.samples, |s| s.gpu);
    let disk = values(&e.samples, |s| s.disk);

    // Processor
    if cpu.len() >= MIN_READINGS {
        let share = share_at_or_above(&cpu, SATURATED_PERCENT);
        if share >= MIN_SATURATED_SHARE {
            report.findings.push(Finding {
                area: Area::Processor,
                certainty: Certainty::Observed,
                headline: format!(
                    "The processor was at or above {SATURATED_PERCENT:.0}% for {} of the readings",
                    percent(share)
                ),
                evidence: vec![
                    format!("average {:.0}%, peak {:.0}%", average(&cpu), peak(&cpu)),
                    format!(
                        "longest unbroken stretch about {} seconds",
                        longest_run_seconds(&e.samples, |s| s.cpu, SATURATED_PERCENT)
                            + e.interval_seconds
                    ),
                ],
                next_steps: vec![
                    (
                        "See which programs are using the processor".into(),
                        Some(Destination::Processes),
                    ),
                    (
                        "Review what starts with Windows".into(),
                        Some(Destination::Startup),
                    ),
                ],
            });
        } else {
            report.unremarkable.push((
                Area::Processor,
                format!("average {:.0}%, peak {:.0}%", average(&cpu), peak(&cpu)),
            ));
        }
    } else {
        report.not_assessed.push((
            "Processor",
            "Windows returned too few processor readings.".into(),
        ));
    }

    // Memory, and a hypothesis when it coincides with unusually high disk throughput.
    if ram.len() >= MIN_READINGS {
        let share = share_at_or_above(&ram, SATURATED_PERCENT);
        if share >= MIN_SATURATED_SHARE {
            report.findings.push(Finding {
                area: Area::Memory,
                certainty: Certainty::Observed,
                headline: format!(
                    "Memory was at or above {SATURATED_PERCENT:.0}% in use for {} of the readings",
                    percent(share)
                ),
                evidence: vec![format!(
                    "average {:.0}%, peak {:.0}%",
                    average(&ram),
                    peak(&ram)
                )],
                next_steps: vec![
                    (
                        "See which programs hold the most memory".into(),
                        Some(Destination::Processes),
                    ),
                    (
                        "Review what starts with Windows".into(),
                        Some(Destination::Startup),
                    ),
                ],
            });
            if disk.len() >= MIN_READINGS {
                let busy_floor = percentile(&disk, 0.75);
                let both = e
                    .samples
                    .iter()
                    .filter(|s| s.ram.is_some_and(|r| r >= SATURATED_PERCENT))
                    .filter(|s| s.disk.is_some_and(|d| d >= busy_floor && busy_floor > 0.0))
                    .count();
                let saturated = e
                    .samples
                    .iter()
                    .filter(|s| s.ram.is_some_and(|r| r >= SATURATED_PERCENT))
                    .count();
                if saturated > 0 && both as f32 / saturated as f32 >= 0.5 {
                    report.findings.push(Finding {
                        area: Area::Memory,
                        certainty: Certainty::Hypothesis,
                        headline: "Windows may have been paging memory to disk".into(),
                        evidence: vec![format!(
                            "{both} of {saturated} full-memory readings also had disk throughput in the busiest quarter of this window"
                        )],
                        next_steps: vec![(
                            "Paging counters were not read, so this is a possible explanation only. Close heavy programs and compare."
                                .into(),
                            Some(Destination::Processes),
                        )],
                    });
                }
            }
        } else {
            report.unremarkable.push((
                Area::Memory,
                format!(
                    "average {:.0}%, peak {:.0}% in use",
                    average(&ram),
                    peak(&ram)
                ),
            ));
        }
    } else {
        report
            .not_assessed
            .push(("Memory", "Windows returned too few memory readings.".into()));
    }

    // Graphics
    if gpu.len() >= MIN_READINGS {
        let share = share_at_or_above(&gpu, GPU_BUSY_PERCENT);
        let cpu_average = if cpu.is_empty() {
            None
        } else {
            Some(average(&cpu))
        };
        if share >= 0.3 && cpu_average.is_some_and(|c| c < 70.0) {
            report.findings.push(Finding {
                area: Area::Graphics,
                certainty: Certainty::Hypothesis,
                headline: "The graphics card may have been the limiting part".into(),
                evidence: vec![
                    format!(
                        "busiest GPU engine at or above {GPU_BUSY_PERCENT:.0}% for {} of the readings",
                        percent(share)
                    ),
                    format!("processor average only {:.0}%", cpu_average.unwrap_or(0.0)),
                ],
                next_steps: vec![(
                    "Lower resolution or graphics settings in the program, then run the check again to compare".into(),
                    None,
                )],
            });
        } else {
            report.unremarkable.push((
                Area::Graphics,
                format!(
                    "busiest engine averaged {:.0}%, peaked {:.0}%",
                    average(&gpu),
                    peak(&gpu)
                ),
            ));
        }
    } else {
        report.not_assessed.push((
            "Graphics",
            "Windows gave no GPU engine counters for this window.".into(),
        ));
    }

    // Storage
    if let Some(free) = e.drive_free_fraction {
        if free < LOW_FREE_SPACE {
            report.findings.push(Finding {
                area: Area::Storage,
                certainty: Certainty::Observed,
                headline: format!("The system drive has only {} free", percent(free)),
                evidence: vec![
                    "Very little free space can slow updates, the page file and temporary files."
                        .into(),
                ],
                next_steps: vec![(
                    "Look for files you can remove".into(),
                    Some(Destination::Cleaner),
                )],
            });
        } else {
            report.unremarkable.push((
                Area::Storage,
                format!("{} of the system drive is free", percent(free)),
            ));
        }
    }

    // Startup
    if let Some(count) = e.startup_enabled {
        if count >= MANY_STARTUP_ITEMS {
            report.findings.push(Finding {
                area: Area::Startup,
                certainty: Certainty::Observed,
                headline: format!("{count} startup items are enabled"),
                evidence: vec![
                    "Whether they slow sign-in or use resources afterwards is not measured here."
                        .into(),
                ],
                next_steps: vec![(
                    "Review the list and disable what you do not need".into(),
                    Some(Destination::Startup),
                )],
            });
        } else {
            report
                .unremarkable
                .push((Area::Startup, format!("{count} startup items enabled")));
        }
    }

    // Background programs
    let mut busy: Vec<&ProcessLoad> = e
        .busy_processes
        .iter()
        .filter(|p| p.cpu_percent >= BUSY_PROCESS_PERCENT)
        .collect();
    busy.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent));
    if !busy.is_empty() {
        report.findings.push(Finding {
            area: Area::BackgroundPrograms,
            certainty: Certainty::Observed,
            headline: "These programs used the most processor time in this window".into(),
            evidence: busy
                .iter()
                .take(5)
                .map(|p| format!("{}: {:.0}% of total processor capacity", p.name, p.cpu_percent))
                .collect(),
            next_steps: vec![(
                "If one of these is not what you were running, it may be competing with it. Open Processes to inspect it.".into(),
                Some(Destination::Processes),
            )],
        });
    } else if !e.busy_processes.is_empty() {
        report.unremarkable.push((
            Area::BackgroundPrograms,
            format!("no program used {BUSY_PROCESS_PERCENT:.0}% or more of the processor"),
        ));
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(cpu: f32, ram: f32, gpu: Option<f32>, disk: f32, n: u64) -> Vec<Sample> {
        (0..n)
            .map(|t| Sample {
                t,
                cpu: Some(cpu),
                ram: Some(ram),
                gpu,
                disk: Some(disk),
                ..Sample::default()
            })
            .collect()
    }

    fn evidence(samples: Vec<Sample>) -> Evidence {
        Evidence {
            source: "test".into(),
            samples,
            interval_seconds: 1,
            ..Evidence::default()
        }
    }

    #[test]
    fn too_few_readings_give_no_findings_instead_of_guesses() {
        let report = analyze(&evidence(run(99.0, 99.0, None, 0.0, 5)));
        assert!(report.too_short);
        assert!(report.findings.is_empty());
    }

    #[test]
    fn sustained_cpu_saturation_is_observed_with_numbers() {
        let report = analyze(&evidence(run(97.0, 40.0, None, 0.0, 30)));
        let finding = &report.findings[0];
        assert_eq!(
            (finding.area, finding.certainty),
            (Area::Processor, Certainty::Observed)
        );
        assert!(finding.headline.contains("100%"));
        assert!(finding.evidence[0].contains("peak 97%"));
        assert!(report.unremarkable.iter().any(|(a, _)| *a == Area::Memory));
    }

    #[test]
    fn a_calm_machine_reports_nothing_wrong() {
        let report = analyze(&evidence(run(20.0, 50.0, Some(30.0), 1.0, 30)));
        assert!(report.findings.is_empty());
        assert_eq!(report.unremarkable.len(), 3);
    }

    #[test]
    fn busy_gpu_with_idle_cpu_is_only_a_hypothesis() {
        let report = analyze(&evidence(run(30.0, 40.0, Some(99.0), 0.0, 30)));
        let finding = report
            .findings
            .iter()
            .find(|f| f.area == Area::Graphics)
            .unwrap();
        assert_eq!(finding.certainty, Certainty::Hypothesis);
        let both_busy = analyze(&evidence(run(95.0, 40.0, Some(99.0), 0.0, 30)));
        assert!(both_busy.findings.iter().all(|f| f.area != Area::Graphics));
    }

    #[test]
    fn full_memory_with_busy_disk_adds_a_paging_hypothesis() {
        let mut samples = run(30.0, 96.0, None, 0.0, 30);
        for (i, s) in samples.iter_mut().enumerate() {
            s.disk = Some(if i % 2 == 0 { 50_000_000.0 } else { 100.0 });
        }
        let report = analyze(&evidence(samples));
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.area == Area::Memory && f.certainty == Certainty::Observed)
        );
        let paging = report
            .findings
            .iter()
            .find(|f| f.certainty == Certainty::Hypothesis)
            .expect("a paging hypothesis");
        assert!(paging.headline.contains("may have been paging"));
        let quiet = analyze(&evidence(run(30.0, 96.0, None, 0.0, 30)));
        assert!(
            quiet
                .findings
                .iter()
                .all(|f| f.certainty == Certainty::Observed)
        );
    }

    #[test]
    fn missing_gpu_counters_are_listed_as_not_assessed() {
        let report = analyze(&evidence(run(10.0, 10.0, None, 0.0, 30)));
        assert!(
            report
                .not_assessed
                .iter()
                .any(|(name, _)| *name == "Graphics")
        );
        assert!(
            report
                .not_assessed
                .iter()
                .any(|(name, _)| *name == "Thermal throttling")
        );
    }

    #[test]
    fn storage_startup_and_process_facts_come_from_the_evidence_given() {
        let mut e = evidence(run(10.0, 10.0, None, 0.0, 30));
        e.drive_free_fraction = Some(0.05);
        e.startup_enabled = Some(22);
        e.busy_processes = vec![
            ProcessLoad {
                name: "idle.exe".into(),
                cpu_percent: 1.0,
                working_set: None,
            },
            ProcessLoad {
                name: "heavy.exe".into(),
                cpu_percent: 40.0,
                working_set: None,
            },
        ];
        let report = analyze(&e);
        let areas: Vec<Area> = report.findings.iter().map(|f| f.area).collect();
        assert!(areas.contains(&Area::Storage));
        assert!(areas.contains(&Area::Startup));
        let background = report
            .findings
            .iter()
            .find(|f| f.area == Area::BackgroundPrograms)
            .unwrap();
        assert_eq!(background.evidence.len(), 1);
        assert!(background.evidence[0].starts_with("heavy.exe"));
    }

    #[test]
    fn unmeasured_areas_are_not_reported_as_fine() {
        let report = analyze(&evidence(run(10.0, 10.0, None, 0.0, 30)));
        assert!(
            report.unremarkable.iter().all(|(a, _)| !matches!(
                a,
                Area::Storage | Area::Startup | Area::BackgroundPrograms
            ))
        );
    }
}
