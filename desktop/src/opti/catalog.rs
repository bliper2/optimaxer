//! The tweak catalog. The 217 tweaks come from Optimaxer's PowerShell catalog, exported once by
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
    /// The PowerShell tweak reads its originals in a -Snapshot script before it changes anything.
    pub snapshot: bool,
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
    /// Tweaks that write the same setting (search style, drive letters, ...) share a group; only one can be selected.
    pub conflict: String,
    /// Why the tweak was retired (no measurable effect, or Windows blocks it). Kept only so an earlier apply can be
    /// undone: hidden unless it has a saved record, never ticked.
    pub retired: String,
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

    /// The rule every automatic plan uses, same as the PowerShell Test-AutoPick. Never: opt-in, retired, Advanced.
    /// Yes: the performance set (tag 'perf', Safe or Moderate; the caller checks its hardware gate), and Safe tweaks
    /// from the recommended set or hardware-gated 'smart' tweaks.
    pub fn auto_pick(&self) -> bool {
        if self.has_tag("optin") || !self.retired.is_empty() || self.risk == "Advanced" {
            return false;
        }
        self.has_tag("perf") || (self.risk == "Safe" && (self.has_tag("safe") || self.has_tag("smart")))
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
        assert_eq!(c.tweaks.len() + c.not_ported.len(), 217);
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
        // the snapshot-based ports must exist, and perf-fth is plain registry now (no FTH list reset)
        for id in ["net-nagle", "perf-timer", "perf-sleepstudy"] {
            assert!(c.find(id).is_some_and(|t| t.special.is_some() && t.script.snapshot), "{id}");
        }
        let fth = c.find("perf-fth").unwrap();
        assert!(fth.special.is_none() && !fth.script.apply && !fth.reg.is_empty());
    }

    #[test]
    fn risk_rule_and_presets_match_the_powershell_catalog() {
        let c = Catalog::load();
        for t in c.tweaks.iter().filter(|t| t.has_tag("safe")) {
            assert_eq!(t.risk, "Safe", "{} is tagged safe", t.id);
        }
        for t in c.tweaks.iter().filter(|t| t.has_tag("perf")) {
            assert!(t.risk != "Advanced" && !t.has_tag("optin") && t.retired.is_empty(), "{} in the performance set", t.id);
        }
        assert!(c.tweaks.iter().filter(|t| !t.retired.is_empty()).count() >= 7);
        for id in ["perf-prefetch", "perf-timer", "perf-cpupower", "perf-priosep", "game-mpo", "game-fso", "net-nagle", "perf-bgapps", "perf-memcompress", "perf-fth"] {
            // a few (memory compression) need PowerShell cmdlets and are listed as not ported
            let Some(t) = c.find(id) else {
                assert!(c.not_ported.iter().any(|s| s.id == id), "{id} missing");
                continue;
            };
            assert!(t.has_tag("optin") && !t.has_tag("safe") && !t.has_tag("gaming") && !t.has_tag("max"), "{id} tags {:?}", t.tags);
            assert!(!t.auto_pick(), "{id}");
        }
        let mut gaming: Vec<_> = c.tweaks.iter().filter(|t| t.has_tag("gaming")).map(|t| t.id.as_str()).collect();
        gaming.sort();
        assert_eq!(gaming, ["game-dvr", "game-mode", "game-mouse", "game-sticky"]);
        assert!(c.find("game-dvr-policy").is_some_and(|t| t.has_tag("optin")));
        assert!(c.tweaks.iter().filter(|t| t.conflict == "search style").count() >= 2);
    }
}
