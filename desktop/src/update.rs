//! Self-update from GitHub releases. The app checks for a newer release when it starts (a small JSON request) and
//! shows a banner. Nothing is downloaded until you press "Update and restart". The zip is verified against the
//! SHA-256 digest GitHub publishes for the asset before anything is replaced; a release without a digest is refused.
//! Windows cannot overwrite a running exe, but it can rename it, so the current exe becomes `<name>.old`, the new one
//! takes its place, and the old file is removed on the next start.

use crate::net;
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const REPO: &str = "bliper2/optimaxer";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
const ASSET_PREFIX: &str = "OrionStore-Optimaxer-desktop-v";
const ASSET_SUFFIX: &str = "-win-x64.zip";
const EXE_NAME: &str = "orion-store-desktop.exe";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: (u32, u32, u32),
    pub tag: String,
    pub notes: String,
    pub page: String,
    pub url: String,
    pub digest: String,
    pub size: u64,
}

impl Release {
    pub fn label(&self) -> String {
        format!("{}.{}.{}", self.version.0, self.version.1, self.version.2)
    }
}

/// `v2.12.0`, `2.12` or `2.12.0-beta` become (2, 12, 0).
pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let core = s.trim().trim_start_matches(['v', 'V']).split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u32>());
    let major = parts.next()?.ok()?;
    let minor = parts.next().unwrap_or(Ok(0)).ok()?;
    let patch = parts.next().unwrap_or(Ok(0)).ok()?;
    Some((major, minor, patch))
}

pub fn current_version() -> (u32, u32, u32) {
    parse_version(CURRENT).unwrap_or((0, 0, 0))
}

/// Picks the Windows desktop zip out of a GitHub release JSON.
pub fn parse_release(v: &Value) -> Result<Release, String> {
    let tag = v["tag_name"].as_str().ok_or("the release has no tag")?.to_string();
    let version = parse_version(&tag).ok_or_else(|| format!("unreadable version tag {tag}"))?;
    let asset = v["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["name"].as_str().is_some_and(|n| n.starts_with(ASSET_PREFIX) && n.ends_with(ASSET_SUFFIX)))
        .ok_or("this release has no desktop download")?;
    Ok(Release {
        version,
        tag,
        notes: v["body"].as_str().unwrap_or("").trim().to_string(),
        page: v["html_url"].as_str().unwrap_or("").to_string(),
        url: asset["browser_download_url"].as_str().ok_or("the desktop download has no link")?.to_string(),
        digest: asset["digest"].as_str().unwrap_or("").to_string(),
        size: asset["size"].as_u64().unwrap_or(0),
    })
}

pub fn is_newer(r: &Release) -> bool {
    r.version > current_version()
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(8))).timeout_recv_response(Some(Duration::from_secs(20))).build().into()
}

/// The latest published release, newer or not.
pub fn latest() -> Result<Release, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let mut resp = agent().get(&url).header("User-Agent", "OrionStore-Optimaxer-Updater").header("Accept", "application/vnd.github+json").call().map_err(|e| format!("could not reach GitHub ({e})"))?;
    let text = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    parse_release(&v)
}

/// Asks GitHub for the latest release. `Ok(None)` means this build is current.
pub fn check() -> Result<Option<Release>, String> {
    let rel = latest()?;
    Ok(is_newer(&rel).then_some(rel))
}

/// `sha256:ABC...` as published by GitHub, lower-cased. `None` when missing or malformed.
pub fn digest_hex(digest: &str) -> Option<String> {
    let h = digest.strip_prefix("sha256:")?.trim().to_lowercase();
    (h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit())).then_some(h)
}

/// The hash line of `certutil -hashfile <file> SHA256`. Older Windows prints it with spaces between bytes.
pub fn parse_certutil(out: &str) -> Option<String> {
    out.lines().map(|l| l.replace(' ', "").trim().to_lowercase()).find(|l| l.len() == 64 && l.chars().all(|c| c.is_ascii_hexdigit()))
}

fn stage_dir(rel: &Release) -> PathBuf {
    let d = net::data_dir().join("update").join(rel.label());
    let _ = fs::create_dir_all(&d);
    d
}

/// Downloads the zip, reporting (done, total) bytes. Stops early when `cancel` returns true.
pub fn download(rel: &Release, progress: &dyn Fn(u64, u64), cancel: &dyn Fn() -> bool) -> Result<PathBuf, String> {
    let zip = stage_dir(rel).join(rel.url.rsplit('/').next().unwrap_or("update.zip"));
    let mut resp = agent().get(&rel.url).header("User-Agent", "OrionStore-Optimaxer-Updater").call().map_err(|e| format!("download failed ({e})"))?;
    let total = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(rel.size);
    let mut reader = resp.body_mut().with_config().limit(512 * 1024 * 1024).reader();
    let part = zip.with_extension("part");
    let mut out = fs::File::create(&part).map_err(|e| e.to_string())?;
    let (mut buf, mut done, mut last) = (vec![0u8; 64 * 1024], 0u64, Instant::now());
    loop {
        if cancel() {
            let _ = fs::remove_file(&part);
            return Err("cancelled".into());
        }
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        if last.elapsed() > Duration::from_millis(80) {
            last = Instant::now();
            progress(done, total);
        }
    }
    drop(out);
    progress(done, total.max(done));
    fs::rename(&part, &zip).map_err(|e| e.to_string())?;
    Ok(zip)
}

/// Refuses the file unless its SHA-256 equals the digest GitHub published for the asset.
pub fn verify(zip: &Path, digest: &str) -> Result<(), String> {
    let want = digest_hex(digest).ok_or("this release has no published checksum, so it was not installed")?;
    let out = Command::new("certutil.exe").args(["-hashfile"]).arg(zip).arg("SHA256").creation_flags(CREATE_NO_WINDOW).output().map_err(|e| format!("could not run certutil ({e})"))?;
    let got = parse_certutil(&String::from_utf8_lossy(&out.stdout)).ok_or("could not read the checksum of the download")?;
    if got == want { Ok(()) } else { Err(format!("the download does not match its published checksum (expected {want}, got {got})")) }
}

/// Unpacks the zip with the `tar` that ships with Windows 10 and later, and returns the new exe.
pub fn extract(zip: &Path, dest: &Path) -> Result<PathBuf, String> {
    let _ = fs::remove_dir_all(dest);
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let out = Command::new("tar.exe").arg("-xf").arg(zip).arg("-C").arg(dest).creation_flags(CREATE_NO_WINDOW).output().map_err(|e| format!("could not run tar ({e})"))?;
    if !out.status.success() {
        return Err(format!("could not unpack the update: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let exe = dest.join(EXE_NAME);
    looks_like_exe(&exe)?;
    Ok(exe)
}

/// A Windows executable starts with "MZ" and is not tiny. Cheap guard against a truncated or wrong file.
pub fn looks_like_exe(p: &Path) -> Result<(), String> {
    let mut f = fs::File::open(p).map_err(|_| format!("the update does not contain {EXE_NAME}"))?;
    let mut head = [0u8; 2];
    f.read_exact(&mut head).map_err(|e| e.to_string())?;
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if &head == b"MZ" && len > 1_000_000 { Ok(()) } else { Err("the update file is not a valid program".into()) }
}

/// Puts `new` in place of `current`. The running exe is renamed to `<current>.old` first; if copying fails it is
/// renamed back. Returns the path of the old file.
pub fn swap_exe(current: &Path, new: &Path) -> Result<PathBuf, String> {
    let old = current.with_extension("exe.old");
    let _ = fs::remove_file(&old);
    fs::rename(current, &old).map_err(|e| format!("could not replace the program (is it installed somewhere that needs administrator rights?): {e}"))?;
    if let Err(e) = fs::copy(new, current) {
        let _ = fs::rename(&old, current);
        return Err(format!("could not write the new version: {e}"));
    }
    Ok(old)
}

/// Removes the previous version left behind by the last update.
pub fn cleanup_old() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = fs::remove_file(exe.with_extension("exe.old"));
    }
    let _ = fs::remove_dir_all(net::data_dir().join("update"));
}

/// Starts the new exe. The caller closes the window afterwards.
pub fn restart(exe: &Path) -> Result<(), String> {
    Command::new(exe).spawn().map(|_| ()).map_err(|e| format!("could not start the new version: {e}"))
}

/// Download, verify, unpack, then swap the exe at `current`. `progress` gets (done, total) while downloading.
pub fn install_to(rel: &Release, current: &Path, progress: &dyn Fn(u64, u64), cancel: &dyn Fn() -> bool) -> Result<(), String> {
    let zip = download(rel, progress, cancel)?;
    verify(&zip, &rel.digest)?;
    let new = extract(&zip, &stage_dir(rel).join("unpacked"))?;
    swap_exe(current, &new).map(|_| ())
}

/// Updates the running exe in place and starts the new version.
pub fn install(rel: &Release, progress: &dyn Fn(u64, u64), cancel: &dyn Fn() -> bool) -> Result<(), String> {
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    install_to(rel, &current, progress, cancel)?;
    restart(&current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample(tag: &str, digest: &str) -> Value {
        json!({
            "tag_name": tag,
            "html_url": "https://github.com/bliper2/optimaxer/releases/tag/v9.9.9",
            "body": "Notes here",
            "assets": [
                { "name": "Optimaxer-v9.9.9.zip", "browser_download_url": "https://x/ps.zip", "digest": "sha256:aa", "size": 10 },
                { "name": "OrionStore-Optimaxer-desktop-v9.9.9-win-x64.zip", "browser_download_url": "https://x/desktop.zip", "digest": digest, "size": 6_000_000 }
            ]
        })
    }

    #[test]
    fn versions_parse_and_compare() {
        assert_eq!(parse_version("v2.12.0"), Some((2, 12, 0)));
        assert_eq!(parse_version("2.12"), Some((2, 12, 0)));
        assert_eq!(parse_version("v3.0.1-beta.2"), Some((3, 0, 1)));
        assert_eq!(parse_version("latest"), None);
        assert!(parse_version("2.10.0").unwrap() > parse_version("2.9.9").unwrap(), "numeric, not text, order");
        assert!(parse_version("v2.12.0").unwrap() > parse_version("2.11.1").unwrap());
    }

    #[test]
    fn this_build_has_a_readable_version() {
        assert!(parse_version(CURRENT).is_some(), "Cargo version {CURRENT}");
    }

    #[test]
    fn release_json_picks_the_desktop_asset_not_the_powershell_zip() {
        let r = parse_release(&sample("v9.9.9", &format!("sha256:{}", "ab".repeat(32)))).unwrap();
        assert_eq!(r.url, "https://x/desktop.zip");
        assert_eq!(r.version, (9, 9, 9));
        assert_eq!(r.size, 6_000_000);
        assert_eq!(r.notes, "Notes here");
        assert!(is_newer(&r));
    }

    #[test]
    fn older_or_equal_releases_are_not_offered() {
        let cur = format!("v{CURRENT}");
        let r = parse_release(&sample(&cur, &format!("sha256:{}", "ab".repeat(32)))).unwrap();
        assert!(!is_newer(&r));
        let old = parse_release(&sample("v0.0.1", "")).unwrap();
        assert!(!is_newer(&old));
    }

    #[test]
    fn a_release_without_a_desktop_zip_is_an_error() {
        let v = json!({ "tag_name": "v9.9.9", "assets": [ { "name": "Optimaxer-v9.9.9.zip", "browser_download_url": "x" } ] });
        assert!(parse_release(&v).is_err());
        assert!(parse_release(&json!({ "assets": [] })).is_err());
    }

    #[test]
    fn digests_must_be_well_formed_sha256() {
        let good = format!("sha256:{}", "AB".repeat(32));
        assert_eq!(digest_hex(&good), Some("ab".repeat(32)));
        assert_eq!(digest_hex(""), None);
        assert_eq!(digest_hex("sha256:abc"), None);
        assert_eq!(digest_hex(&format!("md5:{}", "ab".repeat(32))), None);
    }

    #[test]
    fn certutil_output_is_parsed_with_or_without_spaces() {
        let hash = "0123456789abcdef".repeat(4);
        let new = format!("SHA256 hash of file C:\\x.zip:\r\n{hash}\r\nCertUtil: -hashfile command completed successfully.\r\n");
        assert_eq!(parse_certutil(&new), Some(hash.clone()));
        let spaced: String = hash.as_bytes().chunks(2).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join(" ");
        let old = format!("SHA256 hash of file x:\r\n{spaced}\r\nCertUtil: done\r\n");
        assert_eq!(parse_certutil(&old), Some(hash));
        assert_eq!(parse_certutil("CertUtil: -hashfile command FAILED"), None);
    }

    #[test]
    fn a_release_without_a_checksum_is_refused_before_any_file_is_read() {
        let err = verify(Path::new(r"C:\does\not\exist.zip"), "").unwrap_err();
        assert!(err.contains("no published checksum"), "{err}");
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("opti-update-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn swap_renames_the_old_exe_and_installs_the_new_one() {
        let d = temp("swap");
        let (cur, new) = (d.join("app.exe"), d.join("new.exe"));
        fs::write(&cur, b"old version").unwrap();
        fs::write(&new, b"new version").unwrap();
        let old = swap_exe(&cur, &new).unwrap();
        assert_eq!(fs::read(&cur).unwrap(), b"new version");
        assert_eq!(fs::read(&old).unwrap(), b"old version");
        assert!(old.to_string_lossy().ends_with("app.exe.old"));
    }

    #[test]
    fn a_failed_copy_puts_the_old_exe_back() {
        let d = temp("rollback");
        let cur = d.join("app.exe");
        fs::write(&cur, b"old version").unwrap();
        let err = swap_exe(&cur, &d.join("missing.exe")).unwrap_err();
        assert!(err.contains("could not write"), "{err}");
        assert_eq!(fs::read(&cur).unwrap(), b"old version", "the working program must survive a failed update");
    }

    #[test]
    fn only_real_executables_pass_the_sanity_check() {
        let d = temp("exe");
        let big = d.join("big.exe");
        let mut data = vec![0u8; 1_100_000];
        data[0] = b'M';
        data[1] = b'Z';
        fs::write(&big, &data).unwrap();
        assert!(looks_like_exe(&big).is_ok());
        let small = d.join("small.exe");
        fs::write(&small, b"MZ tiny").unwrap();
        assert!(looks_like_exe(&small).is_err());
        let text = d.join("text.exe");
        fs::write(&text, vec![b'x'; 1_100_000]).unwrap();
        assert!(looks_like_exe(&text).is_err());
        assert!(looks_like_exe(&d.join("nope.exe")).is_err());
    }

    #[test]
    fn certutil_and_tar_verify_and_unpack_a_real_zip() {
        // exercises the two Windows tools the updater depends on, on a throwaway file
        let d = temp("tools");
        let src = d.join("src");
        fs::create_dir_all(&src).unwrap();
        let mut data = vec![7u8; 1_200_000];
        data[0] = b'M';
        data[1] = b'Z';
        fs::write(src.join(EXE_NAME), &data).unwrap();
        let zip = d.join("u.zip");
        let made = Command::new("tar.exe").arg("-a").arg("-cf").arg(&zip).arg("-C").arg(&src).arg(EXE_NAME).creation_flags(CREATE_NO_WINDOW).status();
        if !made.map(|s| s.success()).unwrap_or(false) {
            return; // no tar on this machine: nothing to verify here
        }
        let out = Command::new("certutil.exe").arg("-hashfile").arg(&zip).arg("SHA256").creation_flags(CREATE_NO_WINDOW).output().unwrap();
        let hash = parse_certutil(&String::from_utf8_lossy(&out.stdout)).expect("certutil prints a hash");
        assert!(verify(&zip, &format!("sha256:{hash}")).is_ok());
        assert!(verify(&zip, &format!("sha256:{}", "0".repeat(64))).unwrap_err().contains("does not match"));
        let exe = extract(&zip, &d.join("out")).unwrap();
        assert_eq!(fs::metadata(exe).unwrap().len(), 1_200_000);
    }

    #[test]
    #[ignore = "downloads the latest release from GitHub into a temp folder"]
    fn live_install_updates_a_temp_copy_end_to_end() {
        let rel = latest().expect("latest release");
        let d = temp("live");
        let current = d.join("app.exe");
        let mut stub = vec![0u8; 1_500_000];
        stub[0] = b'M';
        stub[1] = b'Z';
        fs::write(&current, &stub).unwrap();
        install_to(&rel, &current, &|_, _| {}, &|| false).expect("verified install");
        assert!(looks_like_exe(&current).is_ok());
        assert_ne!(fs::metadata(&current).unwrap().len(), 1_500_000, "the stub was replaced by the real exe");
        assert!(d.join("app.exe.old").exists(), "the previous file is kept until the next start");
    }

    #[test]
    #[ignore = "needs network"]
    fn live_check_reaches_github() {
        let r = check();
        println!("{r:?}");
        assert!(r.is_ok());
    }
}
