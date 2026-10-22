//! The tweak catalog. The 216 tweaks come from Optimaxer's PowerShell catalog, exported once by
//! `tools/export-optimaxer.ps1` into `assets/optimaxer/tweaks.json`. Registry, service and scheduled-task tweaks
//! are plain data. Tweaks that ran scripts are mapped to a `Special` port, or listed as not ported.

use super::backend::RegData;
use super::special::{special_for, Special};
use serde::Deserialize;

const EMBEDDED: &str = include_str!("../../assets/optimaxer/tweaks.json");

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum RegVal {
    Num(i64),
    Text(String),
    Bytes(Vec<u8>),
    Multi(Vec<String>),
}

#[derive(Clone, Debug, Deserialize)]
pub struct RegItem {
    pub p: String,
    pub n: String,
    pub v: RegVal,
    #[serde(default)]
    pub t: String,
}

impl RegItem {
    pub fn wanted(&self) -> RegData {
        match (&self.v, self.t.as_str()) {
            (RegVal::Num(n), "QWord") => RegData::Qword(*n as u64),
            (RegVal::Num(n), _) => RegData::Dword((*n & 0xFFFF_FFFF) as u32),
            (RegVal::Text(s), "ExpandString") => RegData::Expand(s.clone()),
            (RegVal::Text(s), _) => RegData::Str(s.clone()),
            (RegVal::Bytes(b), _) => RegData::Bin(b.clone()),
            (RegVal::Multi(m), _) => RegData::Multi(m.clone()),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct SvcItem {
    pub name: String,
    pub start: u32,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ScriptFlags {
    pub apply: bool,
    pub undo: bool,
    pub test: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Tweak {
    pub id: String,
    pub name: String,
    pub desc: String,
    pub cat: String,
    pub risk: String,
    pub tags: Vec<String>,
    pub reg: Vec<RegItem>,
    pub svc: Vec<SvcItem>,
    pub task: Vec<String>,
    pub restart: bool,
    pub explorer: bool,
    pub desktop_only: bool,
    pub ssd_only: bool,
    pub vendor: String,
    pub svc_tol: f64,
    pub min_ram: f64,
    pub need: Vec<String>,
    pub script: ScriptFlags,
    #[serde(skip)]
    pub special: Option<Special>,
    /// Had a PowerShell-only extra step that was not ported; the registry/service/task part still applies.
    #[serde(skip)]
    pub partial: bool,
}

impl Tweak {
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }
}

#[derive(Clone, Debug)]
pub struct Skipped {
    pub id: String,
    pub name: String,
}

#[derive(Default)]
pub struct Catalog {
    pub tweaks: Vec<Tweak>,
    pub not_ported: Vec<Skipped>,
}

/// Tweaks whose registry part is useful and safe without their PowerShell extra (ai-recall also disables the Recall
/// optional feature with DISM, which is not ported).
const PARTIAL_OK: [&str; 1] = ["ai-recall"];

/// Script tweaks that turn out to be a single registry value, so the port needs no script.
fn declarative_override(id: &str) -> Option<Vec<RegItem>> {
    match id {
        // Set-SmbServerConfiguration -EnableSMB1Protocol writes this value
        "net-smb1" => Some(vec![RegItem {
            p: r"HKLM:\SYSTEM\CurrentControlSet\Services\LanmanServer\Parameters".into(),
            n: "SMB1".into(),
            v: RegVal::Num(0),
            t: "DWord".into(),
        }]),
        _ => None,
    }
}

impl Catalog {
    pub fn load() -> Catalog {
        Self::parse(EMBEDDED).unwrap_or_default()
    }

    pub fn parse(json: &str) -> Result<Catalog, String> {
        let all: Vec<Tweak> = serde_json::from_str(json.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
        let mut cat = Catalog::default();
        for mut t in all {
            if t.svc_tol <= 0.0 {
                t.svc_tol = 1.0;
            }
            let scripted = t.script.apply || t.script.undo || t.script.test;
            if let Some(reg) = declarative_override(&t.id) {
                t.reg = reg;
                cat.tweaks.push(t);
                continue;
            }
            t.special = special_for(&t.id);
            if scripted && t.special.is_none() {
                // keep only tweaks whose missing script step is a harmless extra; a script that guards or undoes
                // something (OneDrive, Edge removal) must not run half of itself
                let keep_partial = PARTIAL_OK.contains(&t.id.as_str()) && !(t.reg.is_empty() && t.svc.is_empty() && t.task.is_empty());
                if !keep_partial {
                    cat.not_ported.push(Skipped { id: t.id.clone(), name: t.name.clone() });
                    continue;
                }
                t.partial = true;
            }
            cat.tweaks.push(t);
        }
        Ok(cat)
    }

    pub fn find(&self, id: &str) -> Option<&Tweak> {
        self.tweaks.iter().find(|t| t.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn embedded_catalog_parses_and_ids_are_unique() {
        let c = Catalog::parse(EMBEDDED).unwrap();
        assert_eq!(c.tweaks.len() + c.not_ported.len(), 216);
        let ids: HashSet<_> = c.tweaks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids.len(), c.tweaks.len());
        assert!(c.tweaks.len() > 190, "most of the catalog should be ported, got {}", c.tweaks.len());
    }

    #[test]
    fn every_registry_path_has_a_known_hive() {
        let c = Catalog::load();
        for t in &c.tweaks {
            for r in &t.reg {
                assert!(["HKLM:", "HKCU:", "HKU:", "HKCR:"].iter().any(|h| r.p.starts_with(h)), "{}: {}", t.id, r.p);
                assert!(!r.n.is_empty(), "{} has an empty value name", t.id);
            }
        }
    }

    #[test]
    fn value_types_map_correctly() {
        let c = Catalog::load();
        let t = c.find("perf-shutdown").unwrap();
        assert!(t.reg.iter().any(|r| r.wanted() == RegData::Str("5000".into())));
        let t = c.find("game-mmcss").unwrap();
        assert!(t.reg.iter().any(|r| r.n == "NetworkThrottlingIndex" && r.wanted() == RegData::Dword(0xFFFF_FFFF)));
        let t = c.find("perf-visualfx").unwrap();
        assert!(t.reg.iter().any(|r| matches!(r.wanted(), RegData::Bin(b) if b.len() == 8)));
    }

    #[test]
    fn scripted_tweaks_are_ported_or_reported() {
        let c = Catalog::load();
        assert!(c.find("perf-ultimate").unwrap().special.is_some());
        assert!(c.find("net-smb1").unwrap().special.is_none());
        assert!(!c.find("net-smb1").unwrap().reg.is_empty());
        assert!(c.not_ported.iter().any(|s| s.id == "debloat-onedrive"));
        assert!(c.find("ai-recall").map(|t| t.partial).unwrap_or(false));
    }
}
