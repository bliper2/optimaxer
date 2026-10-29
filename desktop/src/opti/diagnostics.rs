//! Read-only Windows integrity checks. Repair operations remain on the Tools page.

use super::backend::System;

const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Check {
    SystemFiles,
    ComponentStore,
}

impl Check {
    pub fn title(self) -> &'static str {
        match self {
            Self::SystemFiles => "Windows system files",
            Self::ComponentStore => "Windows component store",
        }
    }

    pub fn executable(self) -> &'static str {
        match self {
            Self::SystemFiles => "sfc.exe",
            Self::ComponentStore => "Dism.exe",
        }
    }

    pub fn arguments(self) -> &'static [&'static str] {
        match self {
            Self::SystemFiles => &["/verifyonly"],
            Self::ComponentStore => &["/Online", "/Cleanup-Image", "/ScanHealth"],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Result {
    pub exit_code: i32,
    pub output: String,
}

/// Runs a fixed Windows verification command. The command set deliberately excludes repair switches.
pub fn run(sys: &impl System, check: Check) -> Result {
    let command = sys.run(check.executable(), check.arguments());
    let mut output = command.text;
    if output.len() > MAX_OUTPUT_BYTES {
        let mut end = MAX_OUTPUT_BYTES;
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        output.truncate(end);
        output.push_str("\n\n[Output truncated at 1 MiB]");
    }
    if output.trim().is_empty() {
        output = "The command returned no output.".into();
    }
    Result {
        exit_code: command.code,
        output,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opti::backend::fake::FakeSystem;

    #[test]
    fn checks_use_only_read_only_switches() {
        let sys = FakeSystem::default();
        sys.answer("sfc.exe /verifyonly", "Verification complete.");
        sys.answer(
            "Dism.exe /Online /Cleanup-Image /ScanHealth",
            "No corruption detected.",
        );

        assert_eq!(
            run(&sys, Check::SystemFiles).output,
            "Verification complete."
        );
        assert_eq!(
            run(&sys, Check::ComponentStore).output,
            "No corruption detected."
        );
        assert!(sys.ran("sfc.exe /verifyonly"));
        assert!(sys.ran("Dism.exe /Online /Cleanup-Image /ScanHealth"));
        assert!(!sys.ran("/scannow"));
        assert!(!sys.ran("/RestoreHealth"));
        assert!(
            sys.values.borrow().is_empty(),
            "diagnostic checks must not write registry state"
        );
    }

    #[test]
    fn nonzero_exit_is_preserved_with_output() {
        let sys = FakeSystem::default();
        sys.fail.borrow_mut().insert("sfc.exe".into());
        sys.answer("sfc.exe /verifyonly", "Access denied.");
        let result = run(&sys, Check::SystemFiles);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.output, "Access denied.");
    }

    #[test]
    fn large_output_is_bounded_without_splitting_utf8() {
        let sys = FakeSystem::default();
        sys.answer("sfc.exe /verifyonly", &"é".repeat(MAX_OUTPUT_BYTES));
        let output = run(&sys, Check::SystemFiles).output;
        assert!(output.is_char_boundary(output.len()));
        assert!(output.ends_with("[Output truncated at 1 MiB]"));
        assert!(output.len() <= MAX_OUTPUT_BYTES + 64);
    }
}
