//! Embeds the Optimaxer icon and version information into the Windows executable.
//! This script no-ops unless the *target* OS is Windows, so Linux and other hosts
//! can still check and test the crate. `winresource` stays in `[build-dependencies]`
//! (not `[target.'cfg(windows)'.build-dependencies]`) so a Linux host can still
//! compile the resource when cross-compiling for Windows.

use std::env;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let icon = manifest.join("..").join("assets").join("optimaxer.ico");
    println!("cargo:rerun-if-changed={}", icon.display());

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let icon = icon.canonicalize().expect("assets/optimaxer.ico");
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let mut res = winresource::WindowsResource::new();
    res.set_icon(icon.to_str().expect("icon path is valid UTF-8"));
    res.set("ProductName", "Optimaxer");
    res.set("FileDescription", "Orion Store + Optimaxer");
    res.set("ProductVersion", &version);
    res.set("FileVersion", &version);
    res.set("InternalName", "orion-store-desktop");
    res.set("OriginalFilename", "orion-store-desktop.exe");
    if let Err(error) = res.compile() {
        panic!("failed to embed the Windows icon and version resource: {error}");
    }
}
