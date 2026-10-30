use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub const PC: &str = "PC";
pub const ANDROID: &str = "Android";
pub const TV: &str = "TV";
pub const STREAMS: [&str; 4] = ["Stable", "Beta", "Alpha", "Nightly"];

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppVariant {
    pub arch: String,
    pub url: String,
    pub name: String,
    pub size: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VersionOption {
    pub stream: String,
    pub version: String,
    pub date: String,
    pub variants: Vec<AppVariant>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppItem {
    pub id: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub version: String,
    pub download_url: String,
    pub variants: Vec<AppVariant>,
    pub versions: Vec<VersionOption>,
    pub repo_url: Option<String>,
    pub github_repo: Option<String>,
    pub gitlab_repo: Option<String>,
    pub gitlab_domain: Option<String>,
    pub codeberg_repo: Option<String>,
    pub release_keyword: Option<String>,
    pub package_name: Option<String>,
    pub category: String,
    pub tags: Vec<String>,
    pub platform: String,
    pub size: String,
    pub author: String,
    pub screenshots: Vec<String>,
    pub official_site: Option<String>,
    pub patches: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Catalog {
    pub apps: Vec<AppItem>,
    pub source: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Sort {
    Home,
    Newest,
    NameAsc,
    NameDesc,
    SizeAsc,
    SizeDesc,
}

impl Sort {
    pub const ALL: [Sort; 6] = [
        Sort::Home,
        Sort::Newest,
        Sort::NameAsc,
        Sort::NameDesc,
        Sort::SizeAsc,
        Sort::SizeDesc,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Sort::Home => "Home Page",
            Sort::Newest => "Recently Added",
            Sort::NameAsc => "Name (A-Z)",
            Sort::NameDesc => "Name (Z-A)",
            Sort::SizeAsc => "Size (Smallest)",
            Sort::SizeDesc => "Size (Largest)",
        }
    }
}

fn s(o: &Value, key: &str) -> String {
    match o.get(key) {
        Some(Value::String(v)) => v.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn opt(o: &Value, key: &str) -> Option<String> {
    Some(s(o, key)).filter(|v| !v.is_empty())
}

fn list(o: &Value, key: &str) -> Vec<String> {
    o.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn safe_url(u: &str) -> String {
    let t = u.trim();
    let l = t.to_lowercase();
    if l.starts_with("http://") || l.starts_with("https://") {
        t.to_string()
    } else {
        String::new()
    }
}

fn normalize_platform(p: &str, tags: &[String]) -> String {
    match p.trim().to_lowercase().as_str() {
        "pc" | "desktop" | "windows" => PC.into(),
        "tv" => TV.into(),
        _ => {
            const PC_TAGS: [&str; 5] = ["windows", "pc", "desktop", "win32", "win64"];
            if tags
                .iter()
                .any(|t| PC_TAGS.contains(&t.to_lowercase().as_str()))
            {
                PC.into()
            } else {
                ANDROID.into()
            }
        }
    }
}

fn capitalize(c: &str) -> String {
    let c = c.trim();
    if c.is_empty() {
        return "Utility".into();
    }
    let mut ch = c.chars();
    let first = ch.next().unwrap().to_uppercase().to_string();
    first + &ch.as_str().to_lowercase()
}

pub fn sanitize_app(raw: &Value) -> Option<AppItem> {
    raw.as_object()?;
    let tags = list(raw, "tags");
    let name = Some(s(raw, "name"))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Unknown App".into());
    let package_name = opt(raw, "packageName");
    let id = Some(s(raw, "id"))
        .filter(|v| !v.is_empty())
        .or_else(|| package_name.clone())
        .unwrap_or_else(|| name.clone());
    let version = Some(s(raw, "version"))
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "Latest".into());
    let platform = normalize_platform(&s(raw, "platform"), &tags);
    Some(AppItem {
        id,
        name,
        description: s(raw, "description"),
        icon: safe_url(&s(raw, "icon")),
        version,
        download_url: safe_url(&s(raw, "downloadUrl")),
        variants: Vec::new(),
        versions: Vec::new(),
        repo_url: opt(raw, "repoUrl")
            .map(|u| safe_url(&u))
            .filter(|u| !u.is_empty()),
        github_repo: opt(raw, "githubRepo"),
        gitlab_repo: opt(raw, "gitlabRepo"),
        gitlab_domain: opt(raw, "gitlabDomain"),
        codeberg_repo: opt(raw, "codebergRepo"),
        release_keyword: opt(raw, "releaseKeyword"),
        package_name,
        category: capitalize(&s(raw, "category")),
        tags,
        platform,
        size: s(raw, "size"),
        author: Some(s(raw, "author"))
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "Unknown".into()),
        screenshots: list(raw, "screenshots")
            .iter()
            .map(|u| safe_url(u))
            .filter(|u| !u.is_empty())
            .collect(),
        official_site: opt(raw, "officialSite")
            .map(|u| safe_url(&u))
            .filter(|u| !u.is_empty()),
        patches: list(raw, "patches"),
    })
}

pub fn format_size(bytes: Option<u64>) -> String {
    let Some(b) = bytes.filter(|b| *b > 0) else {
        return "Varies".into();
    };
    let units = ["B", "KB", "MB", "GB", "TB"];
    let i = ((b as f64).ln() / 1024f64.ln()).floor().clamp(0.0, 4.0) as usize;
    let v = (b as f64 / 1024f64.powi(i as i32) * 10.0).round() / 10.0;
    if v.fract() == 0.0 {
        format!("{} {}", v as u64, units[i])
    } else {
        format!("{v} {}", units[i])
    }
}

pub fn parse_size(sz: &str) -> f64 {
    let text = sz.trim().to_lowercase();
    let unit_at = text
        .find(|c: char| c.is_ascii_alphabetic())
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(unit_at);
    let Ok(n) = number.trim().replace(',', "").parse::<f64>() else {
        return 0.0;
    };
    if !n.is_finite() || n < 0.0 {
        return 0.0;
    }
    // Compare every catalog size in MB, including small files and multi-terabyte packages.
    match unit.trim() {
        "b" | "bytes" => n / (1024.0 * 1024.0),
        "kb" | "kib" => n / 1024.0,
        "" | "mb" | "mib" => n,
        "gb" | "gib" => n * 1024.0,
        "tb" | "tib" => n * 1024.0 * 1024.0,
        _ => 0.0,
    }
}

pub fn determine_arch(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if n.contains("arm64") || n.contains("aarch64") || n.contains("v8a") {
        "ARM64"
    } else if n.contains("armeabi") || n.contains("v7a") {
        "ARMv7"
    } else if n.contains("x86_64")
        || n.contains("x64")
        || n.contains("amd64")
        || n.contains("win64")
    {
        "x64"
    } else if n.contains("x86") || n.contains("i686") || n.contains("win32") {
        "x86"
    } else {
        "Universal"
    }
}

fn detect_stream(file: &str, tag: &str, prerelease: bool) -> &'static str {
    let t = format!("{file} {tag}").to_lowercase();
    let has = |ws: &[&str]| ws.iter().any(|w| t.contains(w));
    if has(&["nightly", "daily", "snapshot"]) {
        "Nightly"
    } else if has(&["alpha", "canary", "debug"]) {
        "Alpha"
    } else if has(&["beta", "preview", "rc", "pre"]) || prerelease {
        "Beta"
    } else {
        "Stable"
    }
}

/// First dotted number sequence after stripping release noise, same idea as the Android catalog processor.
pub fn extract_version(input: &str) -> Option<String> {
    const NOISE: [&str; 17] = [
        "armeabi-v7a",
        "arm64-v8a",
        "x86_64",
        "x86",
        "v7a",
        "v8a",
        "-all",
        "_all",
        "-universal",
        "_universal",
        "universal",
        ".apk",
        "release",
        "mod",
        "beta",
        "alpha",
        "nightly",
    ];
    let mut c = input.trim().to_lowercase();
    for n in NOISE {
        c = c.replace(n, "");
    }
    let b: Vec<char> = c.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            let start = i;
            let mut dots = 0;
            while i < b.len()
                && (b[i].is_ascii_digit()
                    || (b[i] == '.' && i + 1 < b.len() && b[i + 1].is_ascii_digit()))
            {
                if b[i] == '.' {
                    dots += 1;
                }
                i += 1;
            }
            if dots > 0 {
                return Some(b[start..i].iter().collect());
            }
        } else {
            i += 1;
        }
    }
    None
}

// region mirror
pub struct Release {
    tag: String,
    name: String,
    prerelease: bool,
    date: String,
    assets: Vec<(String, String, Option<u64>)>,
}

pub fn parse_mirror(root: &Value) -> HashMap<String, Vec<Release>> {
    let mut out = HashMap::new();
    let Some(obj) = root.as_object() else {
        return out;
    };
    for (k, v) in obj {
        let arr: Vec<&Value> = match v {
            Value::Array(a) => a.iter().collect(),
            Value::Object(_) => vec![v],
            _ => continue,
        };
        let releases = arr
            .into_iter()
            .filter(|r| r.is_object())
            .map(|r| Release {
                tag: s(r, "tag_name"),
                name: s(r, "name"),
                prerelease: r
                    .get("prerelease")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                date: Some(s(r, "published_at"))
                    .filter(|d| !d.is_empty())
                    .unwrap_or_else(|| s(r, "created_at")),
                assets: r
                    .get("assets")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter(|e| e.is_object())
                            .map(|e| {
                                (
                                    s(e, "name"),
                                    s(e, "browser_download_url"),
                                    e.get("size").and_then(Value::as_u64),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect();
        out.insert(k.to_lowercase(), releases);
    }
    out
}

fn asset_matches(platform: &str, name: &str, url: &str) -> bool {
    let n = name.to_lowercase();
    let u = url.to_lowercase();
    if platform == PC {
        const EXT: [&str; 8] = [
            ".exe",
            ".msi",
            ".msix",
            ".appx",
            ".zip",
            ".7z",
            ".msixbundle",
            ".appxbundle",
        ];
        let wrong = [
            "linux",
            "macos",
            "darwin",
            "osx",
            ".dmg",
            ".deb",
            ".rpm",
            ".appimage",
            ".apk",
            ".tar",
            "android",
        ];
        EXT.iter().any(|e| n.ends_with(e) || u.ends_with(e)) && !wrong.iter().any(|w| n.contains(w))
    } else {
        n.ends_with(".apk") || u.contains(".apk") || n == "apk" || n.contains("apk")
    }
}

fn arch_rank(platform: &str, arch: &str) -> u8 {
    if platform == PC {
        match arch {
            "x64" => 0,
            "Universal" => 1,
            "x86" => 2,
            "ARM64" => 3,
            _ => 4,
        }
    } else {
        match arch {
            "Universal" => 1,
            "ARM64" => 2,
            "ARMv7" => 3,
            _ => 4,
        }
    }
}

fn date_key(d: &str) -> String {
    d.to_string()
}

fn repo_key(app: &AppItem) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(r) = &app.github_repo {
        keys.push(format!("github::github.com::{}", r.to_lowercase()));
    }
    if let Some(r) = &app.gitlab_repo {
        let d = app
            .gitlab_domain
            .clone()
            .unwrap_or_else(|| "gitlab.com".into());
        keys.push(format!("gitlab::{d}::{}", r.to_lowercase()));
    }
    if let Some(r) = &app.codeberg_repo {
        keys.push(format!("codeberg::codeberg.org::{}", r.to_lowercase()));
    }
    if let Some(u) = &app.repo_url {
        let mut c = u.trim().to_lowercase();
        for p in ["https://www.", "http://www.", "https://", "http://"] {
            if let Some(rest) = c.strip_prefix(p) {
                c = rest.to_string();
                break;
            }
        }
        for h in ["github.com/", "gitlab.com/", "codeberg.org/"] {
            if let Some(rest) = c.strip_prefix(h) {
                c = rest.to_string();
                break;
            }
        }
        let c = c
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .trim_end_matches('/')
            .to_string();
        keys.push(c);
    }
    for r in [&app.github_repo, &app.gitlab_repo, &app.codeberg_repo]
        .into_iter()
        .flatten()
    {
        keys.push(r.to_lowercase());
    }
    keys
}

pub fn process_item(mut app: AppItem, mirror: &HashMap<String, Vec<Release>>) -> AppItem {
    let Some(releases) = repo_key(&app).iter().find_map(|k| mirror.get(k)) else {
        return app;
    };
    let kw = app.release_keyword.as_ref().map(|k| k.to_lowercase());
    let platform = app.platform.clone();
    let mut versions: Vec<(VersionOption, String)> = Vec::new();
    for r in releases {
        let cands: Vec<_> = r
            .assets
            .iter()
            .filter(|(n, u, _)| !safe_url(u).is_empty() && asset_matches(&platform, n, u))
            .collect();
        let assets: Vec<_> = match &kw {
            None => cands,
            Some(k) => {
                let hit: Vec<_> = cands
                    .iter()
                    .filter(|(n, u, _)| {
                        n.to_lowercase().contains(k) || u.to_lowercase().contains(k)
                    })
                    .cloned()
                    .collect();
                if !hit.is_empty() {
                    hit
                } else if r.name.to_lowercase().contains(k) || r.tag.to_lowercase().contains(k) {
                    cands
                } else {
                    Vec::new()
                }
            }
        };
        let Some(first) = assets.first() else {
            continue;
        };
        let stream = detect_stream(&first.0, &r.tag, r.prerelease);
        let tag_v = Some(r.tag.as_str())
            .filter(|t| !matches!(t.to_lowercase().as_str(), "latest" | "all"))
            .and_then(extract_version);
        let date = r.date.split('T').next().unwrap_or("").to_string();
        let version = extract_version(&first.0)
            .or(tag_v)
            .or_else(|| extract_version(&r.name))
            .unwrap_or_else(|| date.clone());
        let mut variants: Vec<AppVariant> = assets
            .iter()
            .map(|(n, u, sz)| AppVariant {
                arch: determine_arch(n).into(),
                url: u.clone(),
                name: n.clone(),
                size: *sz,
            })
            .collect();
        variants.sort_by_key(|v| arch_rank(&platform, &v.arch));
        versions.push((
            VersionOption {
                stream: stream.into(),
                version,
                date,
                variants,
            },
            date_key(&r.date),
        ));
    }
    if versions.is_empty() {
        return app;
    }
    let latest: Vec<VersionOption> = STREAMS
        .iter()
        .filter_map(|st| {
            versions
                .iter()
                .filter(|(v, _)| v.stream == *st)
                .max_by(|a, b| a.1.cmp(&b.1))
                .map(|(v, _)| v.clone())
        })
        .collect();
    let def = latest
        .iter()
        .find(|v| v.stream == "Stable")
        .or(latest.first())
        .cloned()
        .unwrap();
    let v0 = def.variants.first().cloned();
    app.version = def.version.clone();
    app.download_url = v0.as_ref().map(|v| v.url.clone()).unwrap_or_default();
    app.variants = def.variants.clone();
    if let Some(sz) = v0.and_then(|v| v.size) {
        app.size = format_size(Some(sz));
    }
    app.versions = latest;
    app
}

pub fn process(raw: &Value, mirror: &Value) -> Vec<AppItem> {
    let m = parse_mirror(mirror);
    raw.as_array()
        .map(|a| {
            a.iter()
                .filter_map(sanitize_app)
                .map(|app| process_item(app, &m))
                .collect()
        })
        .unwrap_or_default()
}
// endregion

pub fn search(
    q: &str,
    category: &str,
    platform: Option<&str>,
    sort: Sort,
    apps: &[AppItem],
) -> Vec<AppItem> {
    let t = q.trim().to_lowercase();
    let mut out: Vec<AppItem> = if t.is_empty() {
        apps.to_vec()
    } else {
        let mut scored: Vec<(i32, &AppItem)> = apps
            .iter()
            .filter_map(|a| {
                let n = a.name.to_lowercase();
                let sc = if n.contains(&t) { 10 } else { 0 }
                    + if n.starts_with(&t) { 5 } else { 0 }
                    + if a.author.to_lowercase().contains(&t) {
                        5
                    } else {
                        0
                    }
                    + if a.description.to_lowercase().contains(&t) {
                        1
                    } else {
                        0
                    }
                    + if a.id.to_lowercase().contains(&t) {
                        2
                    } else {
                        0
                    };
                (sc > 0).then_some((sc, a))
            })
            .collect();
        scored.sort_by_key(|a| std::cmp::Reverse(a.0));
        scored.into_iter().map(|(_, a)| a.clone()).collect()
    };
    if let Some(p) = platform {
        out.retain(|a| a.platform.eq_ignore_ascii_case(p));
    }
    if category != "All" {
        out.retain(|a| a.category.eq_ignore_ascii_case(category));
    }
    match sort {
        Sort::NameAsc => out.sort_by_key(|a| a.name.to_lowercase()),
        Sort::NameDesc => out.sort_by_key(|a| std::cmp::Reverse(a.name.to_lowercase())),
        Sort::SizeAsc => out.sort_by(|a, b| parse_size(&a.size).total_cmp(&parse_size(&b.size))),
        Sort::SizeDesc => out.sort_by(|a, b| parse_size(&b.size).total_cmp(&parse_size(&a.size))),
        Sort::Newest if t.is_empty() => out.reverse(),
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn size_sort_compares_bytes_through_terabytes() {
        let sizes = ["900 B", "1 KB", "1 MB", "2 GB", "1 TB"];
        let apps: Vec<_> = sizes
            .iter()
            .rev()
            .map(|s| AppItem {
                size: (*s).into(),
                ..Default::default()
            })
            .collect();
        let sorted = search("", "All", None, Sort::SizeAsc, &apps);
        assert_eq!(
            sorted.iter().map(|a| a.size.as_str()).collect::<Vec<_>>(),
            sizes
        );
        assert_eq!(parse_size("1,024 MiB"), 1024.0);
        assert_eq!(parse_size("Varies"), 0.0);
        assert_eq!(parse_size("-5 GB"), 0.0);
    }

    #[test]
    fn catalog_does_not_expose_local_or_executable_links() {
        let raw = json!([{"name": "App", "platform": "PC", "repoUrl": "file:///C:/Windows", "githubRepo": "owner/app"}]);
        let mirror = json!({"owner/app": {"tag_name": "v1.0", "assets": [
            {"name": "bad.exe", "browser_download_url": "file:///C:/bad.exe"},
            {"name": "setup.exe", "browser_download_url": "https://example.com/setup.exe"}
        ]}});
        let apps = process(&raw, &mirror);
        assert!(apps[0].repo_url.is_none());
        assert_eq!(apps[0].variants.len(), 1);
        assert_eq!(apps[0].download_url, "https://example.com/setup.exe");
    }
}
