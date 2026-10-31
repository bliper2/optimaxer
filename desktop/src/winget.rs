use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

const APP_CATALOG_SOURCE: &str = include_str!("../../src/Apps.ps1");
const MAX_EXPORT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_DIAGNOSTIC_BYTES: usize = 4096;
static TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub category: String,
    pub name: String,
    pub id: String,
    pub domain: String,
    pub website: String,
    pub icon_ref: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledPackage {
    pub id: String,
    pub version: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Install,
    Upgrade,
    Uninstall,
    UpgradeAll,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Install => "Install",
            Self::Upgrade => "Upgrade",
            Self::Uninstall => "Uninstall",
            Self::UpgradeAll => "Upgrade all",
        }
    }
}

pub fn catalog() -> Result<Vec<Package>, String> {
    let begin = APP_CATALOG_SOURCE
        .find("$global:OptiApps = @'")
        .ok_or("WinGet catalog start marker is missing")?;
    let body_start = APP_CATALOG_SOURCE[begin..]
        .find('\n')
        .map(|offset| begin + offset + 1)
        .ok_or("WinGet catalog has no rows")?;
    let body = &APP_CATALOG_SOURCE[body_start..];
    let body_end = body
        .find("\n'@ -split")
        .ok_or("WinGet catalog end marker is missing")?;
    parse_catalog(&body[..body_end])
}

fn parse_catalog(raw: &str) -> Result<Vec<Package>, String> {
    let mut packages = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for (line_number, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let fields = line.splitn(6, '|').map(str::trim).collect::<Vec<_>>();
        if fields.len() != 6 || fields[..3].iter().any(|field| field.is_empty()) {
            return Err(format!("Invalid WinGet catalog row {}", line_number + 1));
        }
        let id = fields[2];
        if !valid_package_id(id) || !ids.insert(id.to_ascii_lowercase()) {
            return Err(format!(
                "Invalid or duplicate WinGet package ID on row {}",
                line_number + 1
            ));
        }
        let website = fields[4];
        if !website.is_empty()
            && !(website.starts_with("https://") || website.starts_with("http://"))
        {
            return Err(format!(
                "Invalid website on WinGet catalog row {}",
                line_number + 1
            ));
        }
        packages.push(Package {
            category: fields[0].into(),
            name: fields[1].into(),
            id: id.into(),
            domain: fields[3].into(),
            website: website.into(),
            icon_ref: fields[5].into(),
        });
    }
    if packages.is_empty() {
        return Err("WinGet catalog is empty".into());
    }
    Ok(packages)
}

pub fn valid_package_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 255
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

pub fn parse_export(bytes: &[u8]) -> Result<HashMap<String, InstalledPackage>, String> {
    if bytes.len() as u64 > MAX_EXPORT_BYTES {
        return Err("WinGet export exceeds the 8 MiB limit".into());
    }
    let root: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("WinGet returned invalid export JSON: {error}"))?;
    let sources = root
        .get("Sources")
        .and_then(Value::as_array)
        .ok_or("WinGet export has no Sources array")?;
    let mut installed = HashMap::new();
    for source in sources {
        let Some(packages) = source.get("Packages").and_then(Value::as_array) else {
            continue;
        };
        for package in packages {
            let Some(id) = package
                .get("PackageIdentifier")
                .and_then(Value::as_str)
                .filter(|id| valid_package_id(id))
            else {
                continue;
            };
            let version = package
                .get("Version")
                .and_then(Value::as_str)
                .map(str::to_owned);
            installed.insert(
                id.to_ascii_lowercase(),
                InstalledPackage {
                    id: id.into(),
                    version,
                },
            );
        }
    }
    Ok(installed)
}

fn temp_export_dir() -> Result<PathBuf, String> {
    let temp = std::env::temp_dir();
    for _ in 0..16 {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = temp.join(format!("optimaxer-winget-{}-{id}", std::process::id()));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "Could not create temporary WinGet export folder: {error}"
                ));
            }
        }
    }
    Err("Could not reserve a temporary WinGet export folder".into())
}

fn drain_bounded<R: Read + Send + 'static>(mut reader: R) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) if output.len() < MAX_DIAGNOSTIC_BYTES => {
                    let keep = read.min(MAX_DIAGNOSTIC_BYTES - output.len());
                    output.extend_from_slice(&buffer[..keep]);
                }
                Ok(_) => {}
            }
        }
        output
    })
}

pub fn installed_packages() -> Result<HashMap<String, InstalledPackage>, String> {
    let directory = temp_export_dir()?;
    let output_path = directory.join("installed.json");
    let output = Command::new("winget.exe")
        .args(["export", "--output"])
        .arg(&output_path)
        .args([
            "--source",
            "winget",
            "--include-versions",
            "--disable-interactivity",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let result = match output {
        Err(error) => Err(format!("WinGet is unavailable: {error}")),
        Ok(mut child) => {
            let stdout = child.stdout.take().map(drain_bounded);
            let stderr = child.stderr.take().map(drain_bounded);
            match child.wait() {
                Err(error) => Err(format!("Could not wait for WinGet export: {error}")),
                Ok(status) if !status.success() => {
                    let out = stdout
                        .and_then(|thread| thread.join().ok())
                        .unwrap_or_default();
                    let err = stderr
                        .and_then(|thread| thread.join().ok())
                        .unwrap_or_default();
                    let detail = if err.is_empty() { out } else { err };
                    Err(format!(
                        "WinGet could not list installed packages (exit {}). Source agreements are not accepted automatically. {}",
                        status
                            .code()
                            .map_or_else(|| "unknown".into(), |code| code.to_string()),
                        String::from_utf8_lossy(&detail).trim()
                    ))
                }
                Ok(_) => match fs::metadata(&output_path) {
                    Ok(metadata) if metadata.len() <= MAX_EXPORT_BYTES => {
                        fs::File::open(&output_path)
                            .map_err(|error| format!("Could not open WinGet export: {error}"))
                            .and_then(|file| {
                                let mut bytes = Vec::new();
                                file.take(MAX_EXPORT_BYTES + 1)
                                    .read_to_end(&mut bytes)
                                    .map_err(|error| {
                                        format!("Could not read WinGet export: {error}")
                                    })?;
                                parse_export(&bytes)
                            })
                    }
                    Ok(_) => Err("WinGet export exceeds the 8 MiB limit".into()),
                    Err(error) => Err(format!("WinGet did not create its export file: {error}")),
                },
            }
        }
    };
    let cleanup = fs::remove_dir_all(&directory)
        .map_err(|error| format!("Could not remove temporary WinGet export folder: {error}"));
    match (result, cleanup) {
        (Ok(installed), Ok(())) => Ok(installed),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => Err(format!("{error}; {cleanup}")),
    }
}

pub fn command_args(action: Action, id: Option<&str>) -> Result<Vec<String>, String> {
    let mut args = match action {
        Action::UpgradeAll => vec![
            "upgrade".into(),
            "--all".into(),
            "--source".into(),
            "winget".into(),
        ],
        Action::Install | Action::Upgrade | Action::Uninstall => {
            let id = id
                .filter(|id| valid_package_id(id))
                .ok_or("Invalid WinGet package ID")?;
            let command = match action {
                Action::Install => "install",
                Action::Upgrade => "upgrade",
                Action::Uninstall => "uninstall",
                Action::UpgradeAll => unreachable!(),
            };
            vec![
                command.into(),
                "--id".into(),
                id.into(),
                "--exact".into(),
                "--source".into(),
                "winget".into(),
            ]
        }
    };
    if action == Action::Install {
        args.push("--no-upgrade".into());
    }
    Ok(args)
}

pub fn run_visible(action: Action, id: Option<&str>) -> Result<i32, String> {
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;
    #[cfg(windows)]
    const CREATE_NEW_CONSOLE: u32 = 0x00000010;
    let args = command_args(action, id)?;
    let mut command = Command::new("winget.exe");
    command.args(&args);
    #[cfg(windows)]
    command.creation_flags(CREATE_NEW_CONSOLE);
    let status = command
        .spawn()
        .map_err(|error| format!("Could not start WinGet: {error}"))?
        .wait()
        .map_err(|error| format!("Could not wait for WinGet: {error}"))?;
    Ok(status.code().unwrap_or(-1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curated_catalog_has_unique_valid_ids_and_real_categories() {
        let packages = catalog().unwrap();
        assert!(packages.len() >= 200);
        assert!(packages.iter().any(|package| !package.icon_ref.is_empty()));
        assert!(packages.iter().all(|package| valid_package_id(&package.id)
            && !package.name.is_empty()
            && !package.category.is_empty()));
        assert_eq!(
            packages
                .iter()
                .map(|package| package.id.to_ascii_lowercase())
                .collect::<std::collections::HashSet<_>>()
                .len(),
            packages.len()
        );
    }

    #[test]
    fn export_parser_uses_exact_ids_and_installed_versions() {
        let json = br#"{"Sources":[{"Packages":[{"PackageIdentifier":"Git.Git","Version":"2.50"},{"PackageIdentifier":"Microsoft.PowerToys"}]}]}"#;
        let installed = parse_export(json).unwrap();
        assert_eq!(installed["git.git"].version.as_deref(), Some("2.50"));
        assert_eq!(installed["microsoft.powertoys"].id, "Microsoft.PowerToys");
        assert!(parse_export(b"not json").is_err());
    }

    #[test]
    fn commands_are_exact_scoped_and_never_silent() {
        assert_eq!(
            command_args(Action::Install, Some("Git.Git")).unwrap(),
            [
                "install",
                "--id",
                "Git.Git",
                "--exact",
                "--source",
                "winget",
                "--no-upgrade"
            ]
        );
        assert_eq!(
            command_args(Action::Upgrade, Some("Git.Git")).unwrap()[0],
            "upgrade"
        );
        assert_eq!(
            command_args(Action::Uninstall, Some("Git.Git")).unwrap()[0],
            "uninstall"
        );
        assert_eq!(
            command_args(Action::UpgradeAll, None).unwrap(),
            ["upgrade", "--all", "--source", "winget"]
        );
        assert!(command_args(Action::Install, Some("Git.Git & whoami")).is_err());
    }
}
