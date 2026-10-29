//! Developer aid: with `ORION_SHOTS=<folder>` set, the app walks through its pages, saves each as a PNG of its own
//! window (egui's viewport screenshot, never the desktop), and exits. Nothing here runs in normal use.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

pub struct Shots {
    tx: Sender<Arc<egui::ColorImage>>,
    rx: Receiver<Arc<egui::ColorImage>>,
    dir: PathBuf,
    idx: usize,
    due: f64,
    navigated: bool,
    requested: bool,
}

/// (file name, seconds to wait after navigating before the capture)
pub const STEPS: [(&str, f64); 11] = [
    ("store", 9.0),
    ("store-detail", 3.0),
    ("optimaxer-overview", 14.0),
    ("optimaxer-processes", 4.0),
    ("optimaxer-activity", 3.0),
    ("optimaxer-tweaks", 4.0),
    ("optimaxer-services", 5.0),
    ("optimaxer-startup", 4.0),
    ("optimaxer-cleaner", 3.0),
    ("optimaxer-network", 3.0),
    ("optimaxer-tools", 3.0),
];

impl Shots {
    pub fn from_env() -> Option<Self> {
        let dir = PathBuf::from(std::env::var_os("ORION_SHOTS")?);
        std::fs::create_dir_all(&dir).ok()?;
        let (tx, rx) = channel();
        Some(Shots { tx, rx, dir, idx: 0, due: 0.0, navigated: false, requested: false })
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
        self.due = now + self.step().map_or(0.0, |s| s.1);
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
            if let Err(e) = image::save_buffer(&path, rgba, width as u32, height as u32, image::ColorType::Rgba8) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_advance_navigate_then_capture_then_next() {
        let (tx, rx) = channel();
        let mut s = Shots { tx, rx, dir: std::env::temp_dir(), idx: 0, due: 0.0, navigated: false, requested: false };
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
}
