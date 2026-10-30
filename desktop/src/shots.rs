//! Developer aid: with `ORION_SHOTS=<folder>` set, the app walks through its pages, saves each as a PNG of its own
//! window (egui's viewport screenshot, never the desktop), and exits. Nothing here runs in normal use.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

const ICON_SCREENSHOT_DELAY_SECONDS: f64 = 16.0;

pub struct Shots {
    tx: Sender<Arc<egui::ColorImage>>,
    rx: Receiver<Arc<egui::ColorImage>>,
    dir: PathBuf,
    idx: usize,
    due: f64,
    load_icons: bool,
    navigated: bool,
    requested: bool,
}

/// (file name, seconds to wait after navigating before the capture)
pub const STEPS: [(&str, f64); 19] = [
    ("store", 9.0),
    ("store-detail", 3.0),
    ("store-favorites", 3.0),
    ("winget", 4.0),
    ("optimaxer-overview", 14.0),
    ("optimaxer-smart-optimize", 4.0),
    ("optimaxer-performance", 4.0),
    ("optimaxer-diagnostics", 4.0),
    ("optimaxer-system-snapshot", 4.0),
    ("optimaxer-processes", 4.0),
    ("optimaxer-gaming", 4.0),
    ("optimaxer-activity", 3.0),
    ("optimaxer-tweaks", 4.0),
    ("optimaxer-services", 5.0),
    ("optimaxer-startup", 4.0),
    ("optimaxer-cleaner", 3.0),
    ("optimaxer-network", 3.0),
    ("optimaxer-tools", 3.0),
    ("settings", 3.0),
];

pub fn page_name(slug: &str) -> String {
    slug.strip_prefix("optimaxer-")
        .unwrap_or(slug)
        .replace('-', " ")
}

impl Shots {
    pub fn from_env() -> Option<Self> {
        let dir = PathBuf::from(std::env::var_os("ORION_SHOTS")?);
        std::fs::create_dir_all(&dir).ok()?;
        let load_icons = load_icons_in_screenshot();
        let (tx, rx) = channel();
        Some(Shots {
            tx,
            rx,
            dir,
            idx: 0,
            due: 0.0,
            load_icons,
            navigated: false,
            requested: false,
        })
    }

    pub fn sender(&self) -> Sender<Arc<egui::ColorImage>> {
        self.tx.clone()
    }

    pub fn try_recv(&self) -> Option<Arc<egui::ColorImage>> {
        self.rx.try_recv().ok()
    }

    pub fn step(&self) -> Option<(&'static str, f64)> {
        STEPS.get(self.idx).copied()
    }

    /// True once per step, when the page should be switched to.
    pub fn should_navigate(&mut self, now: f64) -> bool {
        if self.navigated {
            return false;
        }
        self.navigated = true;
        let delay = self.step().map_or(0.0, |(name, seconds)| {
            if name == "winget" && self.load_icons {
                ICON_SCREENSHOT_DELAY_SECONDS
            } else {
                seconds
            }
        });
        self.due = now + delay;
        true
    }

    /// True once per step, when the capture should be requested.
    pub fn should_capture(&mut self, now: f64) -> bool {
        if !self.navigated || self.requested || now < self.due {
            return false;
        }
        self.requested = true;
        true
    }

    pub fn save(&mut self, width: usize, height: usize, rgba: &[u8]) {
        if let Some((name, _)) = self.step() {
            let path = self.dir.join(format!("{name}.png"));
            if let Err(e) = image::save_buffer(
                &path,
                rgba,
                width as u32,
                height as u32,
                image::ColorType::Rgba8,
            ) {
                eprintln!("could not save {}: {e}", path.display());
            }
        }
        self.idx += 1;
        self.navigated = false;
        self.requested = false;
    }

    pub fn finished(&self) -> bool {
        self.idx >= STEPS.len()
    }
}

pub fn load_icons_in_screenshot() -> bool {
    let setting = std::env::var("ORION_SHOTS_LOAD_ICONS").ok();
    screenshot_icons_enabled(setting.as_deref())
}

fn screenshot_icons_enabled(setting: Option<&str>) -> bool {
    !setting.is_some_and(|value| value.eq_ignore_ascii_case("false"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_advance_navigate_then_capture_then_next() {
        let (tx, rx) = channel();
        let mut s = Shots {
            tx,
            rx,
            dir: std::env::temp_dir(),
            idx: 0,
            due: 0.0,
            load_icons: false,
            navigated: false,
            requested: false,
        };
        assert!(!s.should_capture(0.0), "no capture before navigating");
        assert!(s.should_navigate(1.0));
        assert!(!s.should_navigate(1.1), "navigate only once per step");
        assert!(!s.should_capture(2.0), "waits for the page to load");
        assert!(s.should_capture(1.0 + STEPS[0].1));
        assert!(!s.should_capture(100.0), "one capture request per step");
        s.save(1, 1, &[0, 0, 0, 255]);
        assert_eq!(s.step().unwrap().0, "store-detail");
        assert!(!s.finished());
    }

    #[test]
    fn icon_screenshot_waits_for_remote_images() {
        let (tx, rx) = channel();
        let mut s = Shots {
            tx,
            rx,
            dir: std::env::temp_dir(),
            idx: 3,
            due: 0.0,
            load_icons: true,
            navigated: false,
            requested: false,
        };
        assert!(s.should_navigate(10.0));
        assert!(
            !s.should_capture(14.0),
            "remote icon capture keeps the extended wait"
        );
        assert!(s.should_capture(10.0 + ICON_SCREENSHOT_DELAY_SECONDS));

        let (tx, rx) = channel();
        let mut normal = Shots {
            tx,
            rx,
            dir: std::env::temp_dir(),
            idx: 3,
            due: 0.0,
            load_icons: false,
            navigated: false,
            requested: false,
        };
        assert!(normal.should_navigate(10.0));
        assert!(
            normal.should_capture(14.0),
            "normal screenshot keeps the short wait"
        );
    }

    #[test]
    fn screenshot_icons_load_by_default_with_an_offline_opt_out() {
        assert!(screenshot_icons_enabled(None));
        assert!(screenshot_icons_enabled(Some("true")));
        assert!(!screenshot_icons_enabled(Some("false")));
        assert!(!screenshot_icons_enabled(Some("FALSE")));
    }

    #[test]
    fn screenshot_walk_covers_gaming_diagnostics_and_snapshots_pages() {
        assert!(
            STEPS
                .iter()
                .any(|(name, _)| *name == "optimaxer-smart-optimize")
        );
        assert!(STEPS.iter().any(|(name, _)| *name == "optimaxer-gaming"));
        assert!(
            STEPS
                .iter()
                .any(|(name, _)| *name == "optimaxer-diagnostics")
        );
        assert!(
            STEPS
                .iter()
                .any(|(name, _)| *name == "optimaxer-system-snapshot")
        );
        assert!(STEPS.iter().any(|(name, _)| *name == "store-favorites"));
        assert!(STEPS.iter().any(|(name, _)| *name == "winget"));
        assert!(STEPS.iter().any(|(name, _)| *name == "settings"));
    }

    #[test]
    fn screenshot_page_slug_maps_to_the_navigation_label() {
        assert_eq!(page_name("optimaxer-smart-optimize"), "smart optimize");
        assert_eq!(page_name("optimaxer-system-snapshot"), "system snapshot");
        assert_eq!(page_name("optimaxer-performance"), "performance");
    }
}
