//! Versioned tweak profiles. Imports select only tweaks supported by the current catalog and hardware.

use super::catalog::Catalog;
use super::hw::{self, Traits};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

const FORMAT_VERSION: u32 = 1;
const MAX_PROFILE_BYTES: u64 = 64 * 1024;
const MAX_STORE_BYTES: u64 = 256 * 1024;
const MAX_PROFILES: usize = 20;
const MAX_TWEAKS: usize = 512;
const MAX_NAME_CHARS: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    format_version: u32,
    pub name: String,
    pub tweak_ids: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileStore {
    format_version: u32,
    profiles: Vec<Profile>,
}

impl Profile {
    pub fn new(name: &str, tweak_ids: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut tweak_ids = tweak_ids.into_iter().collect::<Vec<_>>();
        tweak_ids.sort();
        let profile = Self {
            format_version: FORMAT_VERSION,
            name: name.trim().to_owned(),
            tweak_ids,
        };
        validate(&profile)?;
        Ok(profile)
    }
}

pub fn compatible_ids(
    profile: &Profile,
    catalog: &Catalog,
    traits: &Traits,
) -> Result<(Vec<String>, usize), String> {
    validate(profile)?;
    let mut selected = Vec::new();
    let mut skipped = 0;
    let mut conflicts = HashSet::new();
    for id in &profile.tweak_ids {
        let Some(tweak) = catalog.find(id) else {
            skipped += 1;
            continue;
        };
        if tweak.has_tag("optin") || !tweak.retired.is_empty() || !hw::fits(tweak, traits) {
            skipped += 1;
            continue;
        }
        if !tweak.conflict.is_empty() && !conflicts.insert(tweak.conflict.as_str()) {
            return Err(format!(
                "Profile contains conflicting tweaks in group '{}'.",
                tweak.conflict
            ));
        }
        selected.push(id.clone());
    }
    Ok((selected, skipped))
}

pub fn difference(left: &Profile, right: &Profile) -> (Vec<String>, Vec<String>) {
    let left_ids: HashSet<&str> = left.tweak_ids.iter().map(String::as_str).collect();
    let right_ids: HashSet<&str> = right.tweak_ids.iter().map(String::as_str).collect();
    let only_left = left
        .tweak_ids
        .iter()
        .filter(|id| !right_ids.contains(id.as_str()))
        .cloned()
        .collect();
    let only_right = right
        .tweak_ids
        .iter()
        .filter(|id| !left_ids.contains(id.as_str()))
        .cloned()
        .collect();
    (only_left, only_right)
}

pub fn load_store(path: &Path) -> Result<Vec<Profile>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Could not read profiles: {error}")),
    };
    if bytes.len() as u64 > MAX_STORE_BYTES {
        return Err("Saved profiles exceed the 256 KiB limit.".into());
    }
    let store: ProfileStore = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Saved profiles are invalid: {error}"))?;
    if store.format_version != FORMAT_VERSION {
        return Err(format!(
            "Saved profiles use unsupported format version {}.",
            store.format_version
        ));
    }
    validate_store(&store.profiles)?;
    Ok(store.profiles)
}

pub fn save_store(path: &Path, profiles: &[Profile]) -> Result<(), String> {
    validate_store(profiles)?;
    let bytes = serde_json::to_vec_pretty(&ProfileStore {
        format_version: FORMAT_VERSION,
        profiles: profiles.to_vec(),
    })
    .map_err(|error| format!("Could not encode profiles: {error}"))?;
    if bytes.len() as u64 > MAX_STORE_BYTES {
        return Err("Saved profiles exceed the 256 KiB limit.".into());
    }
    write_atomic(path, &bytes)
}

pub fn read_export(path: &Path) -> Result<Profile, String> {
    let size = fs::metadata(path)
        .map_err(|error| format!("Could not inspect profile: {error}"))?
        .len();
    if size > MAX_PROFILE_BYTES {
        return Err("Profile exceeds the 64 KiB limit.".into());
    }
    let bytes = fs::read(path).map_err(|error| format!("Could not read profile: {error}"))?;
    if bytes.len() as u64 > MAX_PROFILE_BYTES {
        return Err("Profile exceeds the 64 KiB limit.".into());
    }
    let profile: Profile =
        serde_json::from_slice(&bytes).map_err(|error| format!("Profile is invalid: {error}"))?;
    validate(&profile)?;
    Ok(profile)
}

pub fn write_export(path: &Path, profile: &Profile) -> Result<(), String> {
    validate(profile)?;
    let bytes = serde_json::to_vec_pretty(profile)
        .map_err(|error| format!("Could not encode profile: {error}"))?;
    if bytes.len() as u64 > MAX_PROFILE_BYTES {
        return Err("Profile exceeds the 64 KiB limit.".into());
    }
    write_atomic(path, &bytes)
}

fn validate_store(profiles: &[Profile]) -> Result<(), String> {
    if profiles.len() > MAX_PROFILES {
        return Err(format!(
            "A maximum of {MAX_PROFILES} profiles is supported."
        ));
    }
    let mut names = HashSet::new();
    for profile in profiles {
        validate(profile)?;
        if !names.insert(profile.name.to_lowercase()) {
            return Err(format!("Profile name '{}' is duplicated.", profile.name));
        }
    }
    Ok(())
}

fn validate(profile: &Profile) -> Result<(), String> {
    if profile.format_version != FORMAT_VERSION {
        return Err(format!(
            "Profile uses unsupported format version {}.",
            profile.format_version
        ));
    }
    if profile.name.is_empty()
        || profile.name.trim() != profile.name
        || profile.name.chars().any(char::is_control)
        || profile.name.chars().count() > MAX_NAME_CHARS
    {
        return Err(format!(
            "Profile name must contain 1 to {MAX_NAME_CHARS} characters."
        ));
    }
    if profile.tweak_ids.is_empty() || profile.tweak_ids.len() > MAX_TWEAKS {
        return Err(format!("Profile must contain 1 to {MAX_TWEAKS} tweak IDs."));
    }
    let mut ids = HashSet::new();
    for id in &profile.tweak_ids {
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err("Profile contains an invalid tweak ID.".into());
        }
        if !ids.insert(id) {
            return Err(format!("Profile contains duplicate tweak ID '{id}'."));
        }
    }
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temp = path.with_extension("tmp");
    fs::write(&temp, bytes)
        .map_err(|error| format!("Could not stage {}: {error}", path.display()))?;
    if let Err(error) = replace_file(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(format!("Could not replace {}: {error}", path.display()));
    }
    Ok(())
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // MoveFileExW atomically replaces an existing file on the same volume.
    let flags = windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
        | windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH;
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            flags,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::super::catalog::Tweak;
    use super::*;

    fn profile(name: &str, ids: &[&str]) -> Profile {
        Profile::new(name, ids.iter().map(|id| (*id).to_owned())).unwrap()
    }

    fn tweak(id: &str) -> Tweak {
        Tweak {
            id: id.into(),
            name: id.into(),
            ..Tweak::default()
        }
    }

    fn catalog(tweaks: Vec<Tweak>) -> Catalog {
        Catalog {
            tweaks,
            not_ported: Vec::new(),
        }
    }

    #[test]
    fn portable_profile_round_trips_and_rejects_unknown_versions() {
        let profile = profile("Daily", &["one", "two"]);
        let bytes = serde_json::to_vec(&profile).unwrap();
        let decoded: Profile = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded.name, "Daily");
        assert_eq!(decoded.tweak_ids, ["one", "two"]);
        assert!(Profile::new(" ", ["one".to_owned()]).is_err());
        assert!(Profile::new("Daily", ["one".to_owned(), "one".to_owned()]).is_err());
        assert!(Profile::new("Daily", ["../state".to_owned()]).is_err());
        let unsupported = br#"{"format_version":2,"name":"Future","tweak_ids":["one"]}"#;
        let future: Profile = serde_json::from_slice(unsupported).unwrap();
        assert!(validate(&future)
            .unwrap_err()
            .contains("unsupported format version"));
    }

    #[test]
    fn import_skips_unknown_incompatible_opt_in_and_retired_tweaks() {
        let mut incompatible = tweak("unsupported-here");
        incompatible.need.push("future-hardware".into());
        let mut opt_in = tweak("opt-in");
        opt_in.tags.push("optin".into());
        let mut retired = tweak("retired");
        retired.retired = "obsolete".into();
        let cat = catalog(vec![tweak("fitting"), incompatible, opt_in, retired]);
        let profile = profile(
            "Imported",
            &[
                "fitting",
                "unsupported-here",
                "opt-in",
                "retired",
                "unknown",
            ],
        );

        let (ids, skipped) = compatible_ids(&profile, &cat, &Traits::default()).unwrap();

        assert_eq!(ids, ["fitting"]);
        assert_eq!(skipped, 4);
    }

    #[test]
    fn import_rejects_two_tweaks_that_write_the_same_setting() {
        let mut first = tweak("first");
        first.conflict = "drive-policy".into();
        let mut second = tweak("second");
        second.conflict = "drive-policy".into();
        let cat = catalog(vec![first, second]);

        let error = compatible_ids(
            &profile("Conflict", &["first", "second"]),
            &cat,
            &Traits::default(),
        )
        .unwrap_err();

        assert!(error.contains("conflicting tweaks"));
    }

    #[test]
    fn profile_difference_reports_only_ids_unique_to_each_profile() {
        let left = profile("Gaming", &["shared", "gaming-only", "legacy"]);
        let right = profile("Work", &["shared", "work-only"]);

        assert_eq!(
            difference(&left, &right),
            (
                vec!["gaming-only".to_owned(), "legacy".to_owned()],
                vec!["work-only".to_owned()]
            )
        );
    }

    #[test]
    fn store_round_trips_and_refuses_corrupt_data_without_overwriting_it() {
        let root = std::env::temp_dir().join(format!("optimaxer-profiles-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("profiles.json");
        let profiles = vec![profile("Daily", &["one", "two"])];
        save_store(&path, &profiles).unwrap();
        assert_eq!(load_store(&path).unwrap()[0].name, "Daily");
        let replacement = vec![profile("Work", &["three"])];
        save_store(&path, &replacement).unwrap();
        assert_eq!(load_store(&path).unwrap()[0].name, "Work");

        let export = root.join("portable.json");
        write_export(&export, &profile("Daily", &["one"])).unwrap();
        write_export(&export, &profile("Work", &["two"])).unwrap();
        assert_eq!(read_export(&export).unwrap().name, "Work");

        fs::write(&path, b"{").unwrap();
        assert!(load_store(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{");
        fs::remove_dir_all(root).unwrap();
    }
}
