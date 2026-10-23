//! Hardware profile and tweak selection. Port of Optimaxer's Get-SystemTraits / Test-TweakFits / Get-SmartPlan,
//! reading Win32 and the registry directly instead of WMI: battery flag, physical memory, storage seek-penalty and
//! bus type, GPU driver names, Windows build and board vendor.

use super::backend::{RegData, System};
use super::catalog::{Catalog, Tweak};
use std::collections::{BTreeMap, HashMap, HashSet};
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::UI::Shell::{IsUserAnAdmin, ShellExecuteW};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Traits {
    pub laptop: bool,
    pub ssd: bool,
    pub vendor: String,
    pub ram_gb: f64,
    pub threads: u32,
    pub gpus: Vec<String>,
    pub build: u32,
    pub tags: Vec<String>,
}

impl Traits {
    pub fn has(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }
}

pub struct Plan {
    pub ids: Vec<String>,
    pub notes: Vec<String>,
    /// Why a Moderate or Advanced tweak, or a recommended tweak that does not fit, was left unticked.
    pub skipped: BTreeMap<String, String>,
}

/// Vendors with their own driver/support page. Optimaxer never downloads or installs a driver; the page is opened on request.
pub fn vendor_url(vendor: &str) -> Option<&'static str> {
    match vendor {
        "MSI" => Some("https://www.msi.com/support/download"),
        "Acer" => Some("https://www.acer.com/us-en/support/drivers-and-manuals"),
        "Gigabyte" => Some("https://www.gigabyte.com/Support"),
        _ => None,
    }
}

// region win32 probes
pub fn is_elevated() -> bool {
    // SAFETY: no arguments, returns a BOOL.
    unsafe { IsUserAnAdmin() != 0 }
}

/// Starts this exe again with the UAC "runas" verb. The caller closes the current window when this returns true.
pub fn relaunch_as_admin() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let verb = wide("runas");
    let file = wide(&exe.to_string_lossy());
    // SAFETY: all pointers are NUL-terminated UTF-16 buffers that outlive the call.
    let r = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), 1) };
    r as usize > 32
}

fn has_battery() -> bool {
    let mut s: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    // SAFETY: `s` is a valid out-pointer.
    if unsafe { GetSystemPowerStatus(&mut s) } == 0 {
        return false;
    }
    // 128 = no system battery, 255 = unknown
    s.BatteryFlag != 128 && s.BatteryFlag != 255
}

fn total_ram_gb() -> f64 {
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    // SAFETY: dwLength is set, `m` is a valid out-pointer.
    if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 {
        return 0.0;
    }
    (m.ullTotalPhys as f64 / (1u64 << 30) as f64 * 10.0).round() / 10.0
}

const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D_1400;

/// Runs a standard storage property query against a volume and returns the raw descriptor bytes.
fn storage_query(volume: &str, property_id: u32) -> Option<Vec<u8>> {
    let path: Vec<u16> = format!(r"\\.\{volume}").encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: path is NUL-terminated; access 0 only asks for query rights, so no elevation is needed.
    let h = unsafe { CreateFileW(path.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut()) };
    if h == INVALID_HANDLE_VALUE {
        return None;
    }
    // STORAGE_PROPERTY_QUERY { PropertyId, QueryType = PropertyStandardQuery (0), AdditionalParameters[1] }
    let mut query = [0u8; 12];
    query[..4].copy_from_slice(&property_id.to_le_bytes());
    let mut out = vec![0u8; 64];
    let mut got = 0u32;
    // SAFETY: buffers are valid for the sizes passed; handle is open.
    let ok = unsafe { DeviceIoControl(h, IOCTL_STORAGE_QUERY_PROPERTY, query.as_ptr().cast(), query.len() as u32, out.as_mut_ptr().cast(), out.len() as u32, &mut got, std::ptr::null_mut()) };
    unsafe { CloseHandle(h) };
    (ok != 0).then(|| {
        out.truncate(got as usize);
        out
    })
}

/// (is SSD, is NVMe) for the system drive. Defaults to SSD when Windows will not say.
fn system_disk() -> (bool, bool) {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    // StorageDeviceSeekPenaltyProperty = 7: IncursSeekPenalty byte at offset 8. StorageAdapterProperty = 1: BusType at offset 24.
    let ssd = storage_query(&drive, 7).filter(|d| d.len() > 8).is_none_or(|d| d[8] == 0);
    let nvme = storage_query(&drive, 1).filter(|d| d.len() > 24).is_some_and(|d| d[24] == 17);
    (ssd, nvme)
}
// endregion

pub fn gpu_names(sys: &dyn System) -> Vec<String> {
    let class = r"HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";
    let mut out: Vec<String> = Vec::new();
    for k in sys.reg_subkeys(class) {
        if !k.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if let Some(RegData::Str(n)) = sys.reg_get(&format!(r"{class}\{k}"), "DriverDesc") {
            let l = n.to_lowercase();
            if !n.is_empty() && !l.contains("basic display") && !l.contains("remote display") && !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out
}

fn text(sys: &dyn System, path: &str, name: &str) -> String {
    match sys.reg_get(path, name) {
        Some(RegData::Str(s) | RegData::Expand(s)) => s,
        _ => String::new(),
    }
}

/// Maps the board/system manufacturer to a known brand, or ''.
pub fn vendor_from(names: &str) -> &'static str {
    let n = names.to_lowercase();
    if n.contains("msi") || n.contains("micro-star") {
        "MSI"
    } else if n.contains("acer") {
        "Acer"
    } else if n.contains("gigabyte") || n.contains("giga-byte") {
        "Gigabyte"
    } else {
        ""
    }
}

pub fn detect(sys: &dyn System) -> Traits {
    let laptop = has_battery();
    let (ssd, nvme) = system_disk();
    let ram_gb = total_ram_gb();
    let threads = std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(0);
    let gpus = gpu_names(sys);
    let build = text(sys, r"HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion", "CurrentBuildNumber").parse().unwrap_or(0);
    let bios = r"HKLM:\HARDWARE\DESCRIPTION\System\BIOS";
    let vendor = vendor_from(&format!("{} {}", text(sys, bios, "SystemManufacturer"), text(sys, bios, "BaseBoardManufacturer"))).to_string();
    let tags = tags_for(laptop, ssd, nvme, ram_gb, threads, &gpus, build);
    Traits { laptop, ssd, vendor, ram_gb, threads, gpus, build, tags }
}

/// Trait tags: laptop|desktop, ssd|hdd, nvme, nvidia|amdgpu|intelgpu, dgpu, win11|win10, ram8 (8 GB or less),
/// ram16, ram32, cores8, lowend (weak CPU, little RAM or a hard disk).
pub fn tags_for(laptop: bool, ssd: bool, nvme: bool, ram_gb: f64, threads: u32, gpus: &[String], build: u32) -> Vec<String> {
    let any = |f: &dyn Fn(&str) -> bool| gpus.iter().any(|g| f(&g.to_lowercase()));
    let nvidia = any(&|g| g.contains("nvidia"));
    let amd = any(&|g| g.contains("amd") || g.contains("radeon"));
    let intel = any(&|g| g.contains("intel"));
    // an AMD "Radeon Graphics" or Vega iGPU is not a discrete card
    let discrete_amd = any(&|g| g.contains("radeon rx") || g.contains("radeon pro") || g.contains("arc a") || g.contains("arc b"));
    let dgpu = nvidia || discrete_amd;
    let mut t: Vec<&str> = vec![if laptop { "laptop" } else { "desktop" }, if ssd { "ssd" } else { "hdd" }];
    if nvme {
        t.push("nvme");
    }
    if nvidia {
        t.push("nvidia");
    }
    if amd {
        t.push("amdgpu");
    }
    if intel {
        t.push("intelgpu");
    }
    if dgpu {
        t.push("dgpu");
    }
    t.push(if build >= 22_000 { "win11" } else { "win10" });
    if ram_gb > 0.0 && ram_gb <= 8.5 {
        t.push("ram8");
    }
    if ram_gb >= 15.5 {
        t.push("ram16");
    }
    if ram_gb >= 31.5 {
        t.push("ram32");
    }
    if threads >= 8 {
        t.push("cores8");
    }
    if (ram_gb > 0.0 && ram_gb <= 6.0) || (threads > 0 && threads <= 2) || !ssd {
        t.push("lowend");
    }
    t.into_iter().map(String::from).collect()
}

/// True when a tweak makes sense on this hardware. `need` lists traits that must all be present; '!' means absent.
pub fn fits(t: &Tweak, tr: &Traits) -> bool {
    if t.desktop_only && tr.laptop {
        return false;
    }
    if t.ssd_only && !tr.ssd {
        return false;
    }
    if !t.vendor.is_empty() && t.vendor != tr.vendor {
        return false;
    }
    if t.min_ram > 0.0 && tr.ram_gb > 0.0 && tr.ram_gb < t.min_ram {
        return false;
    }
    t.need.iter().filter(|n| !n.is_empty()).all(|n| match n.strip_prefix('!') {
        Some(neg) => !tr.has(neg),
        None => tr.has(n),
    })
}

/// The hardware fact that rules a tweak out on this PC, in plain words; None when it fits. Same wording as the
/// PowerShell Get-TweakFitReason.
pub fn fit_reason(t: &Tweak, tr: &Traits) -> Option<String> {
    if t.desktop_only && tr.laptop {
        return Some("this PC is a laptop".into());
    }
    if t.ssd_only && !tr.ssd {
        return Some("the system drive is a hard disk".into());
    }
    if !t.vendor.is_empty() && t.vendor != tr.vendor {
        return Some(format!("not a {} system", t.vendor));
    }
    if t.min_ram > 0.0 && tr.ram_gb > 0.0 && tr.ram_gb < t.min_ram {
        return Some(format!("{:.0} GB RAM, needs {:.0} GB", tr.ram_gb, t.min_ram));
    }
    let name = |k: &str| match k {
        "dgpu" => "a discrete GPU".to_string(),
        "hdd" => "a hard disk".into(),
        "ssd" => "an SSD".into(),
        "laptop" => "a laptop".into(),
        "desktop" => "a desktop".into(),
        "lowend" => "low-end hardware".into(),
        "nvidia" => "an NVIDIA GPU".into(),
        "win11" => "Windows 11".into(),
        other => other.to_string(),
    };
    for n in t.need.iter().filter(|n| !n.is_empty()) {
        match n.strip_prefix('!') {
            Some(neg) if tr.has(neg) => return Some(format!("not for {}", name(neg))),
            None if !tr.has(n) => return Some(format!("needs {}", name(n))),
            _ => {}
        }
    }
    None
}

/// Keeps the first id of each conflict group (search style, drive letters, ...): one choice per group.
pub fn resolve_conflicts(cat: &Catalog, ids: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    ids.into_iter()
        .filter(|id| match cat.find(id).map(|t| t.conflict.as_str()).filter(|g| !g.is_empty()) {
            Some(g) => seen.insert(g.to_string()),
            None => true,
        })
        .collect()
}

/// Ids of the tweaks a preset button selects on this machine. Opt-in tweaks never; one per conflict group.
pub fn preset_ids(cat: &Catalog, tr: &Traits, preset: &str) -> Vec<String> {
    let ids = cat.tweaks.iter().filter(|t| !t.has_tag("optin") && fits(t, tr) && t.has_tag(preset)).map(|t| t.id.clone()).collect();
    resolve_conflicts(cat, ids)
}

/// Auto-detect with the same rule as the PowerShell Get-SmartPlan (`Tweak::auto_pick`): Safe-risk recommended
/// tweaks plus Safe 'smart' tweaks whose hardware gate fits. Moderate and Advanced are never ticked, whatever their
/// tags; every one of them gets a reason in `skipped`. Opt-in, unfit and already applied tweaks never appear.
pub fn smart_plan(cat: &Catalog, tr: &Traits, applied: &HashMap<String, bool>) -> Plan {
    let mut ids = Vec::new();
    let mut skipped = BTreeMap::new();
    for t in &cat.tweaks {
        if t.has_tag("optin") {
            continue;
        }
        let why = fit_reason(t, tr);
        if t.risk != "Safe" {
            let r = match &why {
                Some(w) => format!("{} risk, and {w}", t.risk),
                None => format!("{} risk: left for you to tick", t.risk),
            };
            skipped.insert(t.id.clone(), r);
            continue;
        }
        let listed = t.has_tag("safe") || t.has_tag("smart") || t.has_tag("gaming");
        if let Some(w) = why {
            if listed {
                skipped.insert(t.id.clone(), format!("does not fit: {w}"));
            }
            continue;
        }
        if t.auto_pick() && !applied.get(&t.id).copied().unwrap_or(false) {
            ids.push(t.id.clone());
        }
    }
    let ids = resolve_conflicts(cat, ids);
    let mut notes = vec!["Only Safe tweaks are ticked. Moderate and Advanced are left for you to tick.".to_string()];
    notes.push(if tr.laptop { "Laptop: desktop-only tweaks are skipped.".into() } else { "Desktop: no power-plan or CPU-power change is ticked; pick those yourself if you want them.".into() });
    notes.push(if tr.ssd { "SSD: TRIM check included.".into() } else { "Hard disk: SSD-only tweaks are skipped; run a defrag from Windows.".into() });
    if tr.ram_gb > 0.0 {
        notes.push(format!("{:.0} GB RAM: tweaks that need more memory than this are skipped.", tr.ram_gb));
    }
    if tr.has("dgpu") {
        notes.push("Discrete GPU found: gaming tweaks are not ticked automatically; use the Gaming preset if you want them.".into());
    }
    Plan { ids, notes, skipped }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opti::catalog::Catalog;

    fn traits(laptop: bool, ssd: bool, ram: f64, threads: u32, gpus: &[&str]) -> Traits {
        let gpus: Vec<String> = gpus.iter().map(|s| s.to_string()).collect();
        Traits { laptop, ssd, ram_gb: ram, threads, tags: tags_for(laptop, ssd, false, ram, threads, &gpus, 26100), gpus, ..Default::default() }
    }

    #[test]
    fn gaming_desktop_gets_expected_tags() {
        let t = traits(false, true, 63.8, 28, &["NVIDIA GeForce RTX 3090"]);
        for tag in ["desktop", "ssd", "nvidia", "dgpu", "win11", "ram16", "ram32", "cores8"] {
            assert!(t.has(tag), "missing {tag}");
        }
        assert!(!t.has("lowend") && !t.has("laptop"));
    }

    #[test]
    fn amd_integrated_graphics_is_not_a_discrete_gpu() {
        let t = traits(true, true, 16.0, 16, &["AMD Radeon(TM) Graphics"]);
        assert!(t.has("amdgpu") && !t.has("dgpu"));
        let t = traits(false, true, 16.0, 16, &["AMD Radeon RX 7800 XT"]);
        assert!(t.has("dgpu"));
    }

    #[test]
    fn weak_machines_are_lowend() {
        assert!(traits(true, true, 4.0, 4, &[]).has("lowend"));
        assert!(traits(false, false, 16.0, 8, &[]).has("lowend"));
        assert!(traits(true, true, 8.0, 8, &[]).has("ram8"));
    }

    #[test]
    fn vendor_is_normalized() {
        assert_eq!(vendor_from("Micro-Star International Co., Ltd."), "MSI");
        assert_eq!(vendor_from("GIGABYTE"), "Gigabyte");
        assert_eq!(vendor_from("ASUSTeK COMPUTER INC."), "");
    }

    #[test]
    fn fits_respects_hardware_gates() {
        let cat = Catalog::load();
        let desktop = traits(false, true, 64.0, 16, &["NVIDIA GeForce RTX 3090"]);
        let laptop = traits(true, true, 8.0, 8, &["Intel(R) UHD Graphics"]);
        let ultimate = cat.find("perf-ultimate").unwrap();
        assert!(fits(ultimate, &desktop) && !fits(ultimate, &laptop));
        let paging = cat.find("perf-paging").unwrap();
        assert!(fits(paging, &desktop) && !fits(paging, &laptop), "16 GB+ tweak must not fit an 8 GB laptop");
        let hags = cat.find("perf-hags").unwrap();
        assert!(fits(hags, &desktop) && !fits(hags, &laptop), "GPU scheduling needs a discrete GPU");
    }

    #[test]
    fn smart_plan_skips_applied_optin_and_unfit_tweaks() {
        let cat = Catalog::load();
        let tr = traits(true, true, 8.0, 8, &["Intel(R) UHD Graphics"]);
        let none = HashMap::new();
        let plan = smart_plan(&cat, &tr, &none);
        assert!(!plan.ids.is_empty());
        for id in &plan.ids {
            let t = cat.find(id).unwrap();
            assert!(fits(t, &tr) && !t.has_tag("optin"), "{id}");
        }
        assert!(!plan.ids.iter().any(|i| i == "perf-ultimate"), "laptop must not get the Ultimate plan");
        let applied: HashMap<String, bool> = plan.ids.iter().map(|i| (i.clone(), true)).collect();
        assert!(smart_plan(&cat, &tr, &applied).ids.is_empty(), "everything applied means nothing to select");
    }

    #[test]
    fn reference_gaming_desktop_gets_only_safe_tweaks() {
        // i7-14700KF, RTX 3090, 64 GB, Windows 11, in a hot room: no power, timer, Nagle or prefetch changes
        let cat = Catalog::load();
        let pc = traits(false, true, 63.8, 28, &["NVIDIA GeForce RTX 3090"]);
        let plan = smart_plan(&cat, &pc, &HashMap::new());
        assert!(!plan.ids.is_empty());
        for id in ["perf-timer", "perf-cpupower", "net-nagle", "perf-prefetch", "perf-ultimate", "perf-hags", "perf-fth", "game-mode"] {
            assert!(!plan.ids.iter().any(|i| i == id), "{id} was selected");
        }
        for id in &plan.ids {
            let t = cat.find(id).unwrap();
            assert_eq!(t.risk, "Safe", "{id}");
            assert!(t.auto_pick(), "{id}");
        }
        assert!(plan.skipped["perf-hags"].contains("Moderate"));
        assert!(plan.notes.iter().any(|n| n.contains("Moderate and Advanced are left for you to tick")));
        // every Moderate or Advanced tweak that is not opt-in says why it stayed unticked
        for t in cat.tweaks.iter().filter(|t| t.risk != "Safe" && !t.has_tag("optin")) {
            assert!(plan.skipped.contains_key(&t.id), "{} has no reason", t.id);
        }
        // the 5060 Ti answer is the same
        let other = smart_plan(&cat, &traits(false, true, 32.0, 16, &["NVIDIA GeForce RTX 5060 Ti"]), &HashMap::new());
        assert!(other.ids.iter().all(|i| cat.find(i).unwrap().risk == "Safe"));
    }

    #[test]
    fn laptop_reasons_name_the_hardware() {
        let cat = Catalog::load();
        let tr = traits(true, true, 8.0, 8, &["Intel(R) UHD Graphics"]);
        assert_eq!(fit_reason(cat.find("perf-ultimate").unwrap(), &tr).as_deref(), Some("this PC is a laptop"));
        assert!(smart_plan(&cat, &tr, &HashMap::new()).notes.iter().any(|n| n.contains("8 GB RAM")));
    }

    #[test]
    fn presets_never_tick_two_tweaks_of_one_group() {
        let cat = Catalog::load();
        let tr = traits(false, true, 32.0, 16, &["NVIDIA GeForce RTX 3090"]);
        for p in ["max", "safe", "privacy"] {
            let ids = preset_ids(&cat, &tr, p);
            let mut groups = HashSet::new();
            for id in &ids {
                let g = &cat.find(id).unwrap().conflict;
                assert!(g.is_empty() || groups.insert(g.clone()), "{p} ticks two of {g}");
                assert!(!cat.find(id).unwrap().has_tag("optin"));
            }
        }
    }

    #[test]
    fn presets_select_by_tag() {
        let cat = Catalog::load();
        let tr = traits(false, true, 32.0, 16, &["NVIDIA GeForce RTX 3090"]);
        let ids = preset_ids(&cat, &tr, "safe");
        assert!(ids.len() >= 10);
        assert!(ids.iter().all(|i| cat.find(i).unwrap().has_tag("safe")));
    }
}
