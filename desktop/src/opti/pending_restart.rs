//! Reads the registry markers Windows sets when an update or installer needs a restart. Read-only; a missing marker
//! means "not reported", never a guarantee that no restart is needed.

use crate::opti::backend::{RegData, System};

const COMPONENT_SERVICING: &str =
    r"HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending";
const WINDOWS_UPDATE: &str =
    r"HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired";
const SESSION_MANAGER: &str = r"HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager";
const FILE_RENAMES: &str = "PendingFileRenameOperations";

/// Human-readable reasons Windows reports a restart is pending, in a stable order.
pub fn detect(sys: &dyn System) -> Vec<&'static str> {
    let mut reasons = Vec::new();
    if sys.reg_key_exists(COMPONENT_SERVICING) {
        reasons.push("Windows component servicing");
    }
    if sys.reg_key_exists(WINDOWS_UPDATE) {
        reasons.push("Windows Update");
    }
    let renames_pending = match sys.reg_get(SESSION_MANAGER, FILE_RENAMES) {
        Some(RegData::Multi(items)) => items.iter().any(|item| !item.is_empty()),
        Some(_) => true,
        None => false,
    };
    if renames_pending {
        reasons.push("Pending file replacements");
    }
    reasons
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opti::backend::fake::FakeSystem;

    #[test]
    fn a_clean_system_reports_nothing() {
        assert!(detect(&FakeSystem::default()).is_empty());
    }

    #[test]
    fn each_marker_is_reported_once_in_a_stable_order() {
        let sys = FakeSystem::default();
        sys.put(
            SESSION_MANAGER,
            FILE_RENAMES,
            RegData::Multi(vec![r"\??\C:\old.dll".into()]),
        );
        sys.put(WINDOWS_UPDATE, "x", RegData::Dword(1));
        sys.put(COMPONENT_SERVICING, "x", RegData::Dword(1));
        assert_eq!(
            detect(&sys),
            [
                "Windows component servicing",
                "Windows Update",
                "Pending file replacements"
            ]
        );
    }

    #[test]
    fn an_empty_rename_list_is_not_a_pending_restart() {
        let sys = FakeSystem::default();
        sys.put(
            SESSION_MANAGER,
            FILE_RENAMES,
            RegData::Multi(vec![String::new()]),
        );
        assert!(detect(&sys).is_empty());
    }
}
