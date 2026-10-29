//! Portable, validated appearance presets. Presets contain UI preferences only; they never contain system settings.

use crate::net::Settings;
use crate::theme::{self, Palette, Style};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

const FORMAT_VERSION: u32 = 2;
const MAX_PRESET_BYTES: u64 = 16 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    format_version: u32,
    theme: String,
    style: Style,
    palette: Palette,
    custom_accent: String,
    reduce_motion: bool,
    #[serde(default)]
    density: crate::theme::Density,
    #[serde(default)]
    ui_scale: crate::theme::UiScale,
}

impl Preset {
    pub fn capture(settings: &Settings) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            theme: settings.theme.clone(),
            style: settings.style,
            palette: settings.palette,
            custom_accent: settings.custom_accent.clone(),
            reduce_motion: settings.reduce_motion,
            density: settings.density,
            ui_scale: settings.ui_scale,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.format_version, 1 | FORMAT_VERSION) {
            return Err(format!(
                "Unsupported appearance format version {}.",
                self.format_version
            ));
        }
        if !matches!(self.theme.as_str(), "system" | "light" | "dark") {
            return Err("Appearance theme must be system, light or dark.".into());
        }
        if theme::parse_hex_color(&self.custom_accent).is_none() {
            return Err("Custom accent must be a six-digit #RRGGBB color.".into());
        }
        Ok(())
    }

    pub fn apply(&self, settings: &mut Settings) {
        settings.theme.clone_from(&self.theme);
        settings.style = self.style;
        settings.palette = self.palette;
        settings.custom_accent.clone_from(&self.custom_accent);
        settings.reduce_motion = self.reduce_motion;
        settings.density = self.density;
        settings.ui_scale = self.ui_scale;
    }
}

pub fn read(path: &Path) -> Result<Preset, String> {
    let size = fs::metadata(path)
        .map_err(|error| format!("Could not inspect appearance preset: {error}"))?
        .len();
    if size > MAX_PRESET_BYTES {
        return Err("Appearance preset exceeds the 16 KiB limit.".into());
    }
    let bytes =
        fs::read(path).map_err(|error| format!("Could not read appearance preset: {error}"))?;
    if bytes.len() as u64 > MAX_PRESET_BYTES {
        return Err("Appearance preset exceeds the 16 KiB limit.".into());
    }
    let preset: Preset = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Appearance preset is invalid: {error}"))?;
    preset.validate()?;
    Ok(preset)
}

pub fn write(path: &Path, preset: &Preset) -> Result<(), String> {
    preset.validate()?;
    let bytes = serde_json::to_vec_pretty(preset)
        .map_err(|error| format!("Could not encode appearance preset: {error}"))?;
    if bytes.len() as u64 > MAX_PRESET_BYTES {
        return Err("Appearance preset exceeds the 16 KiB limit.".into());
    }
    let temp = path.with_extension("tmp");
    fs::write(&temp, bytes)
        .map_err(|error| format!("Could not stage appearance preset: {error}"))?;
    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(format!("Could not save appearance preset: {error}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_round_trip_and_apply_only_appearance_fields() {
        let source = Settings {
            theme: "dark".into(),
            style: Style::Material,
            palette: Palette::Custom,
            custom_accent: "#12ABEF".into(),
            reduce_motion: true,
            density: crate::theme::Density::Compact,
            ui_scale: crate::theme::UiScale::Larger,
            ..Settings::default()
        };
        let preset = Preset::capture(&source);
        let bytes = serde_json::to_vec(&preset).unwrap();
        let loaded: Preset = serde_json::from_slice(&bytes).unwrap();
        loaded.validate().unwrap();

        let mut destination = Settings {
            download_dir: "keep-this-path".into(),
            auto_update: false,
            ..Settings::default()
        };
        loaded.apply(&mut destination);
        assert_eq!(destination.theme, "dark");
        assert_eq!(destination.style, Style::Material);
        assert_eq!(destination.palette, Palette::Custom);
        assert_eq!(destination.custom_accent, "#12ABEF");
        assert!(destination.reduce_motion);
        assert_eq!(destination.density, crate::theme::Density::Compact);
        assert_eq!(destination.ui_scale, crate::theme::UiScale::Larger);
        assert_eq!(destination.download_dir, "keep-this-path");
        assert!(!destination.auto_update);
    }

    #[test]
    fn invalid_version_mode_and_accent_are_rejected() {
        let mut preset = Preset::capture(&Settings::default());
        preset.format_version = 3;
        assert!(preset.validate().unwrap_err().contains("version"));
        preset.format_version = FORMAT_VERSION;
        preset.theme = "automatic-ish".into();
        assert!(preset.validate().unwrap_err().contains("theme"));
        preset.theme = "system".into();
        preset.custom_accent = "red".into();
        assert!(preset.validate().unwrap_err().contains("accent"));
    }

    #[test]
    fn version_one_appearance_preset_migrates_with_default_density_and_scale() {
        let json = r##"{"format_version":1,"theme":"dark","style":"windows","palette":"system","custom_accent":"#7478F8","reduce_motion":false}"##;
        let preset: Preset = serde_json::from_str(json).unwrap();
        preset.validate().unwrap();
        let mut settings = Settings {
            density: crate::theme::Density::Compact,
            ui_scale: crate::theme::UiScale::Larger,
            ..Settings::default()
        };
        preset.apply(&mut settings);
        assert_eq!(settings.density, crate::theme::Density::Comfortable);
        assert_eq!(settings.ui_scale, crate::theme::UiScale::Default);
    }

    #[test]
    fn file_round_trip_replaces_previous_preset() {
        let dir = std::env::temp_dir().join(format!("optimaxer-appearance-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("appearance.json");
        let first = Preset::capture(&Settings::default());
        write(&path, &first).unwrap();
        let next = Settings {
            theme: "light".into(),
            ..Settings::default()
        };
        write(&path, &Preset::capture(&next)).unwrap();
        assert_eq!(read(&path).unwrap().theme, "light");
        fs::remove_dir_all(dir).unwrap();
    }
}
