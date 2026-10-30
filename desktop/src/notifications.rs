use crate::model;
use crate::net::{DlStatus, Download};
use crate::theme::Tone;
use crate::update::Release;
use std::sync::atomic::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Success,
    Information,
    Warning,
    Error,
    Progress,
}

impl Kind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Success => "Success",
            Self::Information => "Information",
            Self::Warning => "Warning",
            Self::Error => "Error",
            Self::Progress => "In progress",
        }
    }

    pub(crate) fn tone(self) -> Tone {
        match self {
            Self::Success => Tone::Good,
            Self::Information | Self::Progress => Tone::Brand,
            Self::Warning => Tone::Warn,
            Self::Error => Tone::Bad,
        }
    }
}

#[derive(Clone)]
pub(crate) enum Action {
    ReviewActivity,
    OpenDownloads,
    StartUpdate(Release),
    CheckUpdates,
    RevealFile(std::path::PathBuf),
}

#[derive(Clone)]
pub(crate) struct Item {
    pub(crate) id: String,
    pub(crate) kind: Kind,
    pub(crate) title: String,
    pub(crate) detail: String,
    pub(crate) lines: Vec<String>,
    pub(crate) progress: Option<f32>,
    pub(crate) action: Option<Action>,
    pub(crate) dismissible: bool,
    pub(crate) persistent: bool,
}

pub(crate) const MAX_VISIBLE: usize = 100;
pub(crate) const MAX_SAVED_DISMISSALS: usize = 128;

fn operation_id(entry: &crate::opti::history::OperationEntry) -> String {
    let mut identity = format!("{}\0{}\0{}", entry.time, entry.title, entry.status.label());
    for line in &entry.details {
        identity.push('\0');
        identity.push_str(line);
    }
    stable_id("operation", &identity)
}

pub(crate) fn stable_id(kind: &str, identity: &str) -> String {
    // FNV-1a keeps dismissal IDs stable across launches without storing log text in settings.
    let hash = identity
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });
    format!("{kind}:{hash:016x}")
}

pub(crate) fn operation(entry: &crate::opti::history::OperationEntry) -> Option<Item> {
    use crate::opti::history::OperationStatus;

    let (kind, detail) = match entry.status {
        OperationStatus::Warnings => (Kind::Warning, "The worker finished with warnings."),
        OperationStatus::Error => (Kind::Error, "The worker reported an error."),
        OperationStatus::Finished | OperationStatus::Unknown => return None,
    };
    Some(Item {
        id: operation_id(entry),
        kind,
        title: entry.title.clone(),
        detail: detail.into(),
        lines: entry.details.clone(),
        progress: None,
        action: Some(Action::ReviewActivity),
        dismissible: true,
        persistent: true,
    })
}

pub(crate) fn download(download: &Download) -> Option<Item> {
    match download.status() {
        DlStatus::Running => Some(Item {
            id: stable_id("download-running", &download.file),
            kind: Kind::Progress,
            title: format!("Downloading {}", download.file),
            detail: byte_progress_detail(
                download.done.load(Ordering::Relaxed),
                download.total.load(Ordering::Relaxed),
            ),
            lines: Vec::new(),
            progress: download.fraction(),
            action: Some(Action::OpenDownloads),
            dismissible: false,
            persistent: false,
        }),
        DlStatus::Done(path) => Some(Item {
            id: stable_id("download-ready", &path.to_string_lossy()),
            kind: Kind::Success,
            title: "Download ready".into(),
            detail: download.file.clone(),
            lines: vec![path.display().to_string()],
            progress: None,
            action: Some(Action::RevealFile(path)),
            dismissible: true,
            persistent: false,
        }),
        DlStatus::Failed(error) => Some(Item {
            id: stable_id("download-failed", &format!("{}\0{error}", download.file)),
            kind: Kind::Error,
            title: format!("Download failed: {}", download.file),
            detail: "Open Downloads to inspect the failure.".into(),
            lines: vec![error],
            progress: None,
            action: Some(Action::OpenDownloads),
            dismissible: true,
            persistent: false,
        }),
        DlStatus::Cancelled => None,
    }
}

pub(crate) fn byte_progress(done: u64, total: u64) -> Option<f32> {
    if total == 0 {
        None
    } else {
        Some((done as f32 / total as f32).clamp(0.0, 1.0))
    }
}

pub(crate) fn byte_progress_detail(done: u64, total: u64) -> String {
    if total > 0 {
        format!(
            "{} of {} downloaded",
            model::format_size(Some(done)),
            model::format_size(Some(total))
        )
    } else {
        format!(
            "{} downloaded; total size is not available",
            model::format_size(Some(done))
        )
    }
}

pub(crate) fn remember_dismissal(dismissed: &mut Vec<String>, id: &str) {
    if dismissed.iter().any(|item| item == id) {
        return;
    }
    dismissed.push(id.to_owned());
    if dismissed.len() > MAX_SAVED_DISMISSALS {
        let excess = dismissed.len() - MAX_SAVED_DISMISSALS;
        dismissed.drain(..excess);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opti::history::{OperationEntry, OperationStatus};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicU64};

    fn entry(status: OperationStatus, details: Vec<String>) -> OperationEntry {
        OperationEntry {
            time: "2026-10-30T12:30:00Z".into(),
            title: "Reset network".into(),
            status,
            details,
        }
    }

    #[test]
    fn operation_notices_include_worker_warnings_and_errors_only() {
        let details = vec!["ERROR adapter reset failed".into()];
        let failed = entry(OperationStatus::Error, details.clone());
        let notice = operation(&failed).unwrap();
        assert_eq!(notice.kind, Kind::Error);
        assert_eq!(notice.lines, details);
        assert!(matches!(notice.action, Some(Action::ReviewActivity)));
        assert!(operation(&entry(OperationStatus::Warnings, Vec::new())).is_some());
        assert!(operation(&entry(OperationStatus::Finished, Vec::new())).is_none());
        assert!(operation(&entry(OperationStatus::Unknown, Vec::new())).is_none());

        let changed_output = entry(OperationStatus::Error, vec!["ERROR another failure".into()]);
        assert_ne!(operation_id(&failed), operation_id(&changed_output));
        assert_eq!(operation_id(&failed), operation_id(&failed));
    }

    #[test]
    fn transfer_notifications_never_invent_unknown_percentages() {
        assert_eq!(byte_progress(0, 0), None);
        assert_eq!(byte_progress(25, 100), Some(0.25));
        assert!(byte_progress_detail(25, 0).contains("total size is not available"));

        let dl = Download {
            file: "large-file.zip".into(),
            done: AtomicU64::new(25),
            total: AtomicU64::new(0),
            cancel: AtomicBool::new(false),
            status: Mutex::new(DlStatus::Running),
        };
        let notice = download(&dl).unwrap();
        assert_eq!(notice.kind, Kind::Progress);
        assert_eq!(notice.progress, None);
        assert!(matches!(notice.action, Some(Action::OpenDownloads)));
    }

    #[test]
    fn completed_download_notice_reveals_the_real_file() {
        let path = std::path::PathBuf::from("C:/Downloads/tool.zip");
        let dl = Download {
            file: "tool.zip".into(),
            done: AtomicU64::new(10),
            total: AtomicU64::new(10),
            cancel: AtomicBool::new(false),
            status: Mutex::new(DlStatus::Done(path.clone())),
        };
        let notice = download(&dl).unwrap();
        assert_eq!(notice.kind, Kind::Success);
        assert!(matches!(notice.action, Some(Action::RevealFile(p)) if p == path));
    }

    #[test]
    fn dismissal_ids_are_idempotent_and_bounded() {
        let mut dismissed = Vec::new();
        remember_dismissal(&mut dismissed, "first");
        remember_dismissal(&mut dismissed, "first");
        assert_eq!(dismissed, ["first"]);
        for i in 0..=MAX_SAVED_DISMISSALS {
            remember_dismissal(&mut dismissed, &format!("item-{i}"));
        }
        assert_eq!(dismissed.len(), MAX_SAVED_DISMISSALS);
        assert!(!dismissed.contains(&"first".into()));
    }
}
