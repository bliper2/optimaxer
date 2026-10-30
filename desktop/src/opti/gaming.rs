//! Per-game Windows graphics preferences with a saved-original, conflict-safe undo path.

use super::backend::{RegData, System};
use super::profiles::write_atomic;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::Path;

const FORMAT_VERSION: u32 = 1;
const MAX_PROFILES: usize = 50;
const MAX_STORE_BYTES: u64 = 128 * 1024;
const MAX_EXPORT_BYTES: u64 = 4 * 1024;
const MAX_LIBRARY_FILE_BYTES: u64 = 1024 * 1024;
const MAX_LIBRARY_ROOTS: usize = 32;
const MAX_MANIFEST_FILES: usize = 2048;
const MAX_DISCOVERED_GAMES: usize = 500;
pub const GPU_PREFERENCES_KEY: &str = r"HKCU:\Software\Microsoft\DirectX\UserGpuPreferences";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibrarySource {
    Steam,
    Epic,
}

impl LibrarySource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Steam => "Steam",
            Self::Epic => "Epic Games",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredGame {
    pub name: String,
    pub source: LibrarySource,
    pub install_dir: std::path::PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LibraryScan {
    pub games: Vec<DiscoveredGame>,
    pub warnings: Vec<String>,
}

/// Read installed games from Steam app manifests and Epic launcher manifests.
/// The scan is bounded and never walks game directories or changes launcher data.
pub fn scan_libraries(steam_roots: &[std::path::PathBuf], epic_manifests: &Path) -> LibraryScan {
    let mut scan = LibraryScan::default();
    let mut roots = std::collections::HashSet::new();
    let mut libraries_scanned = 0;
    for root in steam_roots.iter().take(MAX_LIBRARY_ROOTS) {
        let Ok(root) = root.canonicalize() else {
            continue;
        };
        if !roots.insert(root.clone()) {
            continue;
        }
        let library_file = root.join("steamapps").join("libraryfolders.vdf");
        let Ok(contents) = read_bounded_text(&library_file) else {
            continue;
        };
        let libraries = vdf_library_paths(&contents, &root);
        for library in libraries.into_iter().take(MAX_LIBRARY_ROOTS) {
            if libraries_scanned >= MAX_LIBRARY_ROOTS {
                break;
            }
            libraries_scanned += 1;
            scan_steam_library(&library, &mut scan);
            if scan.games.len() >= MAX_DISCOVERED_GAMES {
                break;
            }
        }
        if scan.games.len() >= MAX_DISCOVERED_GAMES {
            break;
        }
    }
    scan_epic_manifests(epic_manifests, &mut scan);
    scan.games.sort_by_key(|game| game.name.to_lowercase());
    scan.games
        .dedup_by(|left, right| left.install_dir == right.install_dir);
    scan
}

fn read_bounded_text(path: &Path) -> Result<String, String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_LIBRARY_FILE_BYTES {
        return Err("manifest is not a regular file within the 1 MiB size limit".into());
    }
    use std::io::Read;
    let mut contents = String::new();
    fs::File::open(path)
        .and_then(|file| {
            file.take(MAX_LIBRARY_FILE_BYTES + 1)
                .read_to_string(&mut contents)
        })
        .map_err(|error| error.to_string())?;
    if contents.len() as u64 > MAX_LIBRARY_FILE_BYTES {
        return Err("manifest exceeds the 1 MiB size limit".into());
    }
    Ok(contents)
}

fn scan_steam_library(library: &Path, scan: &mut LibraryScan) {
    let apps = library.join("steamapps");
    let Ok(entries) = fs::read_dir(&apps) else {
        return;
    };
    for entry in entries.flatten().take(MAX_MANIFEST_FILES) {
        if scan.games.len() >= MAX_DISCOVERED_GAMES {
            break;
        }
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !file_name.starts_with("appmanifest_") || !file_name.ends_with(".acf") {
            continue;
        }
        let Ok(contents) = read_bounded_text(&path) else {
            continue;
        };
        let fields = vdf_top_level_fields(&contents);
        let (Some(name), Some(install_dir)) = (fields.get("name"), fields.get("installdir")) else {
            continue;
        };
        if name.trim().is_empty()
            || install_dir.is_empty()
            || install_dir == "."
            || install_dir == ".."
            || Path::new(install_dir).components().count() != 1
        {
            continue;
        }
        let install_dir = apps.join("common").join(install_dir);
        if let Ok(install_dir) = install_dir.canonicalize() {
            let install_dir = strip_windows_verbatim_prefix(install_dir);
            scan.games.push(DiscoveredGame {
                name: name.clone(),
                source: LibrarySource::Steam,
                install_dir,
            });
        }
    }
}

fn strip_windows_verbatim_prefix(path: std::path::PathBuf) -> std::path::PathBuf {
    path.strip_prefix(r"\\?\")
        .map_or(path.clone(), Path::to_path_buf)
}

fn scan_epic_manifests(manifests: &Path, scan: &mut LibraryScan) {
    let Ok(entries) = fs::read_dir(manifests) else {
        return;
    };
    for entry in entries.flatten().take(MAX_MANIFEST_FILES) {
        if scan.games.len() >= MAX_DISCOVERED_GAMES {
            break;
        }
        let path = entry.path();
        if path
            .extension()
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("item"))
        {
            continue;
        }
        let Ok(contents) = read_bounded_text(&path) else {
            scan.warnings.push(format!(
                "Skipped an unreadable Epic manifest: {}",
                path.display()
            ));
            continue;
        };
        let Ok(item) = serde_json::from_str::<Value>(&contents) else {
            scan.warnings.push(format!(
                "Skipped an invalid Epic manifest: {}",
                path.display()
            ));
            continue;
        };
        let Some(name) = item
            .get("DisplayName")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
        else {
            continue;
        };
        let Some(location) = item
            .get("InstallLocation")
            .and_then(Value::as_str)
            .filter(|location| !location.is_empty())
        else {
            continue;
        };
        let install_dir = Path::new(location);
        if install_dir.is_dir() {
            scan.games.push(DiscoveredGame {
                name: name.to_owned(),
                source: LibrarySource::Epic,
                install_dir: install_dir.to_path_buf(),
            });
        }
    }
}

fn vdf_tokens(contents: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = contents.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            let mut token = String::new();
            while let Some(ch) = chars.next() {
                match ch {
                    '"' => break,
                    '\\' => match chars.next() {
                        Some(next) => token.push(next),
                        None => break,
                    },
                    _ => token.push(ch),
                }
            }
            tokens.push(token);
        }
    }
    tokens
}

fn vdf_top_level_fields(contents: &str) -> std::collections::HashMap<String, String> {
    let tokens = vdf_tokens(contents);
    tokens
        .windows(2)
        .filter_map(|pair| {
            let key = pair[0].to_lowercase();
            matches!(key.as_str(), "name" | "installdir" | "path").then(|| (key, pair[1].clone()))
        })
        .collect()
}

fn vdf_library_paths(contents: &str, root: &Path) -> Vec<std::path::PathBuf> {
    let mut libraries = vec![root.to_path_buf()];
    let tokens = vdf_tokens(contents);
    for pair in tokens.windows(2) {
        let legacy_library_index =
            !pair[0].is_empty() && pair[0].chars().all(|ch| ch.is_ascii_digit());
        if pair[0].eq_ignore_ascii_case("path") || legacy_library_index {
            let path = Path::new(&pair[1]);
            if path.is_absolute() {
                libraries.push(path.to_path_buf());
            }
        }
    }
    libraries.sort();
    libraries.dedup();
    libraries
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphicsPreference {
    #[default]
    Windows,
    PowerSaving,
    HighPerformance,
}

impl GraphicsPreference {
    pub fn label(self) -> &'static str {
        match self {
            Self::Windows => "Windows decides",
            Self::PowerSaving => "Power saving",
            Self::HighPerformance => "High performance",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameProfile {
    pub executable: String,
    pub preference: GraphicsPreference,
    #[serde(default)]
    undo: Option<UndoRecord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UndoRecord {
    before: Option<RegistrySnapshot>,
    applied: Option<RegistrySnapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistrySnapshot {
    value_type: String,
    value: Value,
}

impl RegistrySnapshot {
    fn capture(data: &RegData) -> Self {
        Self {
            value_type: data.type_name().to_owned(),
            value: data.to_json(),
        }
    }

    fn restore(&self) -> Result<RegData, String> {
        if self.value_type != "String" || !self.value.is_string() {
            return Err("Saved Windows GPU preference is not a plain registry string.".into());
        }
        RegData::from_json(&self.value, &self.value_type)
            .ok_or_else(|| "Saved Windows graphics preference is unreadable.".into())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Store {
    format_version: u32,
    profiles: Vec<GameProfile>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortablePreference {
    format_version: u32,
    preference: GraphicsPreference,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileState {
    WindowsDefault,
    Selected,
    Available,
    RestorePending,
    ChangedElsewhere,
}

impl GameProfile {
    pub fn new(executable: &str) -> Result<Self, String> {
        validate_executable(executable)?;
        Ok(Self {
            executable: executable.to_owned(),
            preference: GraphicsPreference::Windows,
            undo: None,
        })
    }

    pub fn can_remove(&self) -> bool {
        self.undo.is_none()
    }

    pub fn has_saved_original(&self) -> bool {
        self.undo.is_some()
    }
}

pub fn store_path() -> std::path::PathBuf {
    super::state::state_dir().join("games.json")
}

pub fn validate_executable(executable: &str) -> Result<(), String> {
    let path = Path::new(executable);
    if !path.is_absolute()
        || !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        return Err("Choose an absolute path to a Windows .exe file.".into());
    }
    if executable.chars().any(char::is_control) || executable.len() > 32_000 {
        return Err("The executable path contains unsupported characters or is too long.".into());
    }
    Ok(())
}

pub fn load_store(path: &Path) -> Result<Vec<GameProfile>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Could not read game profiles: {error}")),
    };
    if bytes.len() as u64 > MAX_STORE_BYTES {
        return Err("Game profiles exceed the 128 KiB limit.".into());
    }
    let store: Store = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Saved game profiles are invalid: {error}"))?;
    validate_store(&store)?;
    Ok(store.profiles)
}

pub fn save_store(path: &Path, profiles: &[GameProfile]) -> Result<(), String> {
    let store = Store {
        format_version: FORMAT_VERSION,
        profiles: profiles.to_vec(),
    };
    validate_store(&store)?;
    let bytes = serde_json::to_vec_pretty(&store)
        .map_err(|error| format!("Could not encode game profiles: {error}"))?;
    if bytes.len() as u64 > MAX_STORE_BYTES {
        return Err("Game profiles exceed the 128 KiB limit.".into());
    }
    write_atomic(path, &bytes)
}

pub fn write_export(path: &Path, profile: &GameProfile) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(&PortablePreference {
        format_version: FORMAT_VERSION,
        preference: profile.preference,
    })
    .map_err(|error| format!("Could not encode graphics preference: {error}"))?;
    write_atomic(path, &bytes)
}

pub fn read_export(path: &Path) -> Result<GraphicsPreference, String> {
    let size = fs::metadata(path)
        .map_err(|error| format!("Could not inspect graphics profile: {error}"))?
        .len();
    if size > MAX_EXPORT_BYTES {
        return Err("Graphics profile exceeds the 4 KiB limit.".into());
    }
    let bytes =
        fs::read(path).map_err(|error| format!("Could not read graphics profile: {error}"))?;
    if bytes.len() as u64 > MAX_EXPORT_BYTES {
        return Err("Graphics profile exceeds the 4 KiB limit.".into());
    }
    let export: PortablePreference = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Graphics profile is invalid: {error}"))?;
    if export.format_version != FORMAT_VERSION {
        return Err(format!(
            "Graphics profile uses unsupported format version {}.",
            export.format_version
        ));
    }
    Ok(export.preference)
}

pub fn state(sys: &impl System, profile: &GameProfile) -> Result<ProfileState, String> {
    let current = sys.reg_get(GPU_PREFERENCES_KEY, &profile.executable);
    if let Some(undo) = &profile.undo {
        let applied = undo
            .applied
            .as_ref()
            .map(RegistrySnapshot::restore)
            .transpose()?;
        let before = undo
            .before
            .as_ref()
            .map(RegistrySnapshot::restore)
            .transpose()?;
        return Ok(if current == applied {
            ProfileState::Selected
        } else if current == before {
            ProfileState::RestorePending
        } else {
            ProfileState::ChangedElsewhere
        });
    }
    let selected = preference_matches(&current, profile.preference)?;
    Ok(if selected {
        if profile.preference == GraphicsPreference::Windows {
            ProfileState::WindowsDefault
        } else {
            ProfileState::Selected
        }
    } else {
        ProfileState::Available
    })
}

pub fn preview(
    sys: &impl System,
    profile: &GameProfile,
) -> Result<(Option<RegData>, Option<RegData>), String> {
    if profile.undo.is_some() {
        return Err("Restore the saved Windows value before applying this profile again.".into());
    }
    let current = sys.reg_get(GPU_PREFERENCES_KEY, &profile.executable);
    let desired = edit_preference(current.as_ref(), profile.preference)?;
    if current == desired {
        return Err("This graphics preference is already selected.".into());
    }
    Ok((current, desired))
}

pub fn undo_preview(
    sys: &impl System,
    profile: &GameProfile,
) -> Result<(Option<RegData>, Option<RegData>), String> {
    let undo = profile
        .undo
        .as_ref()
        .ok_or("No Optimaxer original is saved for this game.")?;
    let current = sys.reg_get(GPU_PREFERENCES_KEY, &profile.executable);
    let before = undo
        .before
        .as_ref()
        .map(RegistrySnapshot::restore)
        .transpose()?;
    let applied = undo
        .applied
        .as_ref()
        .map(RegistrySnapshot::restore)
        .transpose()?;
    if current != applied && current != before {
        return Err(
            "Windows graphics preference changed outside Optimaxer. Saved originals are protected."
                .into(),
        );
    }
    Ok((current, before))
}

/// Saves the exact previous registry value before changing Windows. The preview value is checked again here.
pub fn apply(
    sys: &impl System,
    store_path: &Path,
    profiles: &mut [GameProfile],
    executable: &str,
    expected_current: &Option<RegData>,
) -> Result<(), String> {
    let index = profile_index(profiles, executable)?;
    if profiles[index].undo.is_some() {
        return Err("Restore or inspect the pending saved-original state before applying this profile again.".into());
    }
    let current = sys.reg_get(GPU_PREFERENCES_KEY, executable);
    if &current != expected_current {
        return Err(
            "Windows graphics preference changed after preview. Review it again before applying."
                .into(),
        );
    }
    let desired = edit_preference(current.as_ref(), profiles[index].preference)?;
    if current == desired {
        return Err("This graphics preference is already selected.".into());
    }

    profiles[index].undo = Some(UndoRecord {
        before: current.as_ref().map(RegistrySnapshot::capture),
        applied: desired.as_ref().map(RegistrySnapshot::capture),
    });
    if let Err(error) = save_store(store_path, profiles) {
        profiles[index].undo = None;
        return Err(error);
    }
    write_value(sys, executable, desired.as_ref())?;
    if sys.reg_get(GPU_PREFERENCES_KEY, executable) != desired {
        return Err("Windows did not retain the requested graphics preference. Saved originals remain available for recovery.".into());
    }
    Ok(())
}

pub fn undo(
    sys: &impl System,
    store_path: &Path,
    profiles: &mut [GameProfile],
    executable: &str,
) -> Result<(), String> {
    let index = profile_index(profiles, executable)?;
    let undo = profiles[index]
        .undo
        .clone()
        .ok_or("No Optimaxer change is saved for this game.")?;
    let before = undo
        .before
        .as_ref()
        .map(RegistrySnapshot::restore)
        .transpose()?;
    let applied = undo
        .applied
        .as_ref()
        .map(RegistrySnapshot::restore)
        .transpose()?;
    let current = sys.reg_get(GPU_PREFERENCES_KEY, executable);
    if current != before {
        if current != applied {
            return Err("Windows graphics preference changed outside Optimaxer. Saved originals were kept; review the current value before restoring.".into());
        }
        write_value(sys, executable, before.as_ref())?;
        if sys.reg_get(GPU_PREFERENCES_KEY, executable) != before {
            return Err("Windows did not restore the original graphics preference. Saved originals remain available for retry.".into());
        }
    }
    profiles[index].undo = None;
    if let Err(error) = save_store(store_path, profiles) {
        profiles[index].undo = Some(undo);
        return Err(error);
    }
    Ok(())
}

pub fn set_preference(
    store_path: &Path,
    profiles: &mut [GameProfile],
    executable: &str,
    preference: GraphicsPreference,
) -> Result<(), String> {
    let index = profile_index(profiles, executable)?;
    if profiles[index].undo.is_some() {
        return Err("Restore the active graphics preference before changing this profile.".into());
    }
    let previous = profiles[index].preference;
    profiles[index].preference = preference;
    if let Err(error) = save_store(store_path, profiles) {
        profiles[index].preference = previous;
        return Err(error);
    }
    Ok(())
}

pub fn remove(
    store_path: &Path,
    profiles: &mut Vec<GameProfile>,
    executable: &str,
) -> Result<(), String> {
    let index = profile_index(profiles, executable)?;
    if profiles[index].undo.is_some() {
        return Err(
            "Restore the saved Windows preference before removing this game profile.".into(),
        );
    }
    let removed = profiles.remove(index);
    if let Err(error) = save_store(store_path, profiles) {
        profiles.insert(index, removed);
        return Err(error);
    }
    Ok(())
}

fn write_value(sys: &impl System, executable: &str, value: Option<&RegData>) -> Result<(), String> {
    match value {
        Some(value) => sys.reg_set(GPU_PREFERENCES_KEY, executable, value),
        None => sys.reg_delete_value(GPU_PREFERENCES_KEY, executable),
    }
}

fn preference_matches(
    current: &Option<RegData>,
    preference: GraphicsPreference,
) -> Result<bool, String> {
    let current = match current {
        None => return Ok(preference == GraphicsPreference::Windows),
        Some(RegData::Str(value)) => value,
        Some(value) => {
            return Err(format!(
                "Windows GPU preference has unsupported registry type {}.",
                value.type_name()
            ));
        }
    };
    let mode = gpu_preference(current)?;
    Ok(match preference {
        GraphicsPreference::Windows => mode.is_none() || mode == Some(0),
        GraphicsPreference::PowerSaving => mode == Some(1),
        GraphicsPreference::HighPerformance => mode == Some(2),
    })
}

fn gpu_preference(value: &str) -> Result<Option<u8>, String> {
    let mut found = None;
    for part in value.split(';').filter(|part| !part.trim().is_empty()) {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("GpuPreference") {
            if found.is_some() {
                return Err(
                    "Windows GPU preference contains duplicate GpuPreference directives.".into(),
                );
            }
            let mode = value.trim().parse::<u8>().map_err(|_| {
                "Windows GPU preference contains an unreadable GpuPreference value.".to_string()
            })?;
            if mode > 2 {
                return Err(
                    "Windows GPU preference contains an unsupported GpuPreference value.".into(),
                );
            }
            found = Some(mode);
        }
    }
    Ok(found)
}

fn edit_preference(
    current: Option<&RegData>,
    preference: GraphicsPreference,
) -> Result<Option<RegData>, String> {
    let raw = match current {
        None => "",
        Some(RegData::Str(value)) => value,
        Some(value) => {
            return Err(format!(
                "Windows GPU preference has unsupported registry type {}.",
                value.type_name()
            ));
        }
    };
    gpu_preference(raw)?;
    // Keep other Windows directives byte-for-byte; this value can hold more than the GPU preference.
    let mut retained = raw
        .split_inclusive(';')
        .filter(|part| {
            let directive = part.trim().trim_end_matches(';');
            let key = directive.split_once('=').map(|(key, _)| key.trim());
            !key.is_some_and(|key| key.eq_ignore_ascii_case("GpuPreference"))
        })
        .collect::<String>();
    let mode = match preference {
        GraphicsPreference::Windows => None,
        GraphicsPreference::PowerSaving => Some(1),
        GraphicsPreference::HighPerformance => Some(2),
    };
    if let Some(mode) = mode {
        if !retained.is_empty() && !retained.ends_with(';') {
            retained.push(';');
        }
        retained.push_str(&format!("GpuPreference={mode};"));
    }
    Ok((!retained.is_empty()).then_some(RegData::Str(retained)))
}

fn profile_index(profiles: &[GameProfile], executable: &str) -> Result<usize, String> {
    profiles
        .iter()
        .position(|profile| profile.executable.eq_ignore_ascii_case(executable))
        .ok_or_else(|| "Game profile was removed or changed. Refresh and try again.".into())
}

fn validate_store(store: &Store) -> Result<(), String> {
    if store.format_version != FORMAT_VERSION {
        return Err(format!(
            "Game profiles use unsupported format version {}.",
            store.format_version
        ));
    }
    if store.profiles.len() > MAX_PROFILES {
        return Err(format!(
            "A maximum of {MAX_PROFILES} game profiles is supported."
        ));
    }
    let mut paths = std::collections::HashSet::new();
    for profile in &store.profiles {
        validate_executable(&profile.executable)?;
        if !paths.insert(profile.executable.to_lowercase()) {
            return Err("Saved game profiles contain a duplicate executable path.".into());
        }
        if let Some(undo) = &profile.undo {
            if let Some(value) = &undo.before {
                value.restore()?;
            }
            if let Some(value) = &undo.applied {
                value.restore()?;
            }
            if undo.before == undo.applied {
                return Err("Saved game profile has an invalid unchanged undo record.".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opti::backend::fake::FakeSystem;
    use std::path::PathBuf;

    const EXE: &str = r"C:\Games\Example\game.exe";

    fn temp() -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("opti-gaming-{}", crate::opti::now_stamp()));
        fs::create_dir_all(&dir).unwrap();
        let store = dir.join("games.json");
        (dir, store)
    }

    fn profile() -> GameProfile {
        let mut profile = GameProfile::new(EXE).unwrap();
        profile.preference = GraphicsPreference::HighPerformance;
        profile
    }

    #[test]
    fn apply_and_undo_restore_exact_existing_value() {
        let (dir, store) = temp();
        let sys = FakeSystem::default();
        sys.put(
            GPU_PREFERENCES_KEY,
            EXE,
            RegData::Str("VRROptimizeEnable=0;".into()),
        );
        let mut profiles = vec![profile()];
        let before = sys.reg_get(GPU_PREFERENCES_KEY, EXE);
        apply(&sys, &store, &mut profiles, EXE, &before).unwrap();
        assert_eq!(
            sys.reg_get(GPU_PREFERENCES_KEY, EXE),
            Some(RegData::Str("VRROptimizeEnable=0;GpuPreference=2;".into()))
        );
        assert_eq!(state(&sys, &profiles[0]).unwrap(), ProfileState::Selected);
        undo(&sys, &store, &mut profiles, EXE).unwrap();
        assert_eq!(sys.reg_get(GPU_PREFERENCES_KEY, EXE), before);
        assert!(profiles[0].can_remove());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn apply_to_windows_default_removes_value_and_undo_recreates_exact_value() {
        let (dir, store) = temp();
        let sys = FakeSystem::default();
        let mut profile = profile();
        profile.preference = GraphicsPreference::Windows;
        sys.put(
            GPU_PREFERENCES_KEY,
            EXE,
            RegData::Str("GpuPreference=2;".into()),
        );
        let mut profiles = vec![profile];
        let before = sys.reg_get(GPU_PREFERENCES_KEY, EXE);
        apply(&sys, &store, &mut profiles, EXE, &before).unwrap();
        assert_eq!(sys.reg_get(GPU_PREFERENCES_KEY, EXE), None);
        undo(&sys, &store, &mut profiles, EXE).unwrap();
        assert_eq!(sys.reg_get(GPU_PREFERENCES_KEY, EXE), before);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn changing_gpu_preference_preserves_other_windows_directives() {
        let (dir, store) = temp();
        let sys = FakeSystem::default();
        let mut profile = profile();
        profile.preference = GraphicsPreference::PowerSaving;
        let before = RegData::Str("VRROptimizeEnable=0;GpuPreference=2;".into());
        sys.put(GPU_PREFERENCES_KEY, EXE, before.clone());
        let mut profiles = vec![profile];
        apply(&sys, &store, &mut profiles, EXE, &Some(before.clone())).unwrap();
        assert_eq!(
            sys.reg_get(GPU_PREFERENCES_KEY, EXE),
            Some(RegData::Str("VRROptimizeEnable=0;GpuPreference=1;".into()))
        );
        undo(&sys, &store, &mut profiles, EXE).unwrap();
        assert_eq!(sys.reg_get(GPU_PREFERENCES_KEY, EXE), Some(before));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn external_change_blocks_undo_and_preserves_snapshot() {
        let (dir, store) = temp();
        let sys = FakeSystem::default();
        let mut profiles = vec![profile()];
        let before = None;
        apply(&sys, &store, &mut profiles, EXE, &before).unwrap();
        sys.put(
            GPU_PREFERENCES_KEY,
            EXE,
            RegData::Str("GpuPreference=1;".into()),
        );
        assert!(
            undo(&sys, &store, &mut profiles, EXE)
                .unwrap_err()
                .contains("outside Optimaxer")
        );
        assert!(!profiles[0].can_remove());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn preview_drift_and_registry_denial_do_not_report_success() {
        let (dir, store) = temp();
        let sys = FakeSystem::default();
        let mut profiles = vec![profile()];
        sys.put(
            GPU_PREFERENCES_KEY,
            EXE,
            RegData::Str("GpuPreference=1;".into()),
        );
        assert!(
            apply(&sys, &store, &mut profiles, EXE, &None)
                .unwrap_err()
                .contains("changed after preview")
        );
        assert!(profiles[0].can_remove());
        sys.deny_writes
            .borrow_mut()
            .insert(GPU_PREFERENCES_KEY.to_lowercase());
        let before = sys.reg_get(GPU_PREFERENCES_KEY, EXE);
        assert!(apply(&sys, &store, &mut profiles, EXE, &before).is_err());
        assert_eq!(
            state(&sys, &profiles[0]).unwrap(),
            ProfileState::RestorePending
        );
        assert!(!profiles[0].can_remove());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_store_and_invalid_executable_are_rejected() {
        let (dir, store) = temp();
        fs::write(&store, b"not json").unwrap();
        assert!(load_store(&store).unwrap_err().contains("invalid"));
        assert!(GameProfile::new("relative.exe").is_err());
        assert!(GameProfile::new(r"C:\Games\readme.txt").is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_profile_save_never_changes_registry_or_loses_memory_state() {
        let (dir, store) = temp();
        let invalid_store = dir.join("missing-parent").join("games.json");
        let sys = FakeSystem::default();
        let mut profiles = vec![profile()];
        assert!(apply(&sys, &invalid_store, &mut profiles, EXE, &None).is_err());
        assert!(profiles[0].can_remove());
        assert_eq!(sys.reg_get(GPU_PREFERENCES_KEY, EXE), None);

        let valid_store = store;
        save_store(&valid_store, &profiles).unwrap();
        let before = None;
        apply(&sys, &valid_store, &mut profiles, EXE, &before).unwrap();
        assert!(undo(&sys, &invalid_store, &mut profiles, EXE).is_err());
        assert!(
            profiles[0].has_saved_original(),
            "failed journal cleanup remains retryable in memory"
        );
        assert_eq!(
            sys.reg_get(GPU_PREFERENCES_KEY, EXE),
            None,
            "the registry restore ran before persistence failed"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn portable_export_contains_only_the_preference() {
        let (dir, export) = temp();
        let profile = profile();
        write_export(&export, &profile).unwrap();
        let text = fs::read_to_string(&export).unwrap();
        assert!(
            !text.contains(EXE),
            "machine-specific executable paths are not exported"
        );
        assert!(
            !text.contains("undo"),
            "machine-specific originals are not exported"
        );
        assert_eq!(
            read_export(&export).unwrap(),
            GraphicsPreference::HighPerformance
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn scans_installed_steam_and_epic_manifests_without_entering_game_folders() {
        let (dir, _) = temp();
        let steam = dir.join("Steam");
        let steam_apps = steam.join("steamapps");
        let game_dir = steam_apps.join("common").join("Example Game");
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(
            steam_apps.join("libraryfolders.vdf"),
            "\"libraryfolders\" { \"0\" { \"path\" \"ignored root\" } }",
        )
        .unwrap();
        fs::write(
            steam_apps.join("appmanifest_10.acf"),
            "\"AppState\" { \"name\" \"Example Game\" \"installdir\" \"Example Game\" }",
        )
        .unwrap();

        let epic_manifests = dir.join("EpicManifests");
        let epic_dir = dir.join("EpicGame");
        fs::create_dir_all(&epic_manifests).unwrap();
        fs::create_dir_all(&epic_dir).unwrap();
        fs::write(
            epic_manifests.join("example.item"),
            serde_json::json!({
                "DisplayName": "Epic Example",
                "InstallLocation": epic_dir
            })
            .to_string(),
        )
        .unwrap();

        let scan = scan_libraries(std::slice::from_ref(&steam), &epic_manifests);
        assert_eq!(scan.games.len(), 2);
        assert!(
            scan.games
                .iter()
                .any(|game| game.source == LibrarySource::Steam
                    && game.install_dir.ends_with(Path::new("Example Game")))
        );
        assert!(
            scan.games
                .iter()
                .any(|game| game.source == LibrarySource::Epic && game.install_dir == epic_dir)
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn steam_manifest_install_directory_must_be_one_safe_component() {
        let (dir, _) = temp();
        let steam = dir.join("Steam");
        let apps = steam.join("steamapps");
        fs::create_dir_all(apps.join("common")).unwrap();
        fs::write(apps.join("libraryfolders.vdf"), "\"libraryfolders\" {}").unwrap();
        fs::write(
            apps.join("appmanifest_11.acf"),
            "\"name\" \"Invalid\" \"installdir\" \"..\\outside\"",
        )
        .unwrap();
        let scan = scan_libraries(&[steam], &dir.join("missing-epic"));
        assert!(scan.games.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn vdf_parser_reads_escaped_library_paths() {
        let paths = vdf_library_paths(
            "\"libraryfolders\" { \"0\" { \"path\" \"C:\\\\Games\\\\SteamLibrary\" } }",
            Path::new(r"C:\Steam"),
        );
        assert!(
            paths
                .iter()
                .any(|path| path == Path::new(r"C:\Games\SteamLibrary"))
        );
        let legacy = vdf_library_paths(
            "\"libraryfolders\" { \"1\" \"D:\\\\SteamLibrary\" }",
            Path::new(r"C:\Steam"),
        );
        assert!(
            legacy
                .iter()
                .any(|path| path == Path::new(r"D:\SteamLibrary"))
        );
    }
}
