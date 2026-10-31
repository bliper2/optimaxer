//! Linux edition of the Optimaxer section. Everything that reads or changes a Linux system lives here: `/proc` and
//! `/sys` telemetry, systemd units, XDG autostart, sysctl and config-file tweaks, the cleaner and DNS.
//!
//! The parsers and the apply/undo logic are plain functions over a `System` trait, so they also compile on Windows
//! and their tests run there. Only `RealSystem` and the `/proc` readers are Unix-only.
#![cfg_attr(windows, allow(dead_code))]

pub mod cleaner;
pub mod network;
pub mod processes;
pub mod procfs;
pub mod services;
pub mod startup;
pub mod store;
pub mod sysmon;
pub mod system;
pub mod tweaks;

/// `1.5 GB` style size, binary units.
pub fn fmt_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Quote one argument for `/bin/sh`.
pub fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_use_binary_units() {
        assert_eq!(fmt_size(0), "0 B");
        assert_eq!(fmt_size(1023), "1023 B");
        assert_eq!(fmt_size(1536), "1.5 KB");
        assert_eq!(fmt_size(5 * 1024 * 1024 * 1024), "5.0 GB");
    }

    #[test]
    fn shell_quoting_survives_single_quotes() {
        assert_eq!(sh_quote("a b"), "'a b'");
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
    }
}
