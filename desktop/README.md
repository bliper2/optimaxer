# Orion Store + Optimaxer for Windows

One Windows window, one exe. The Orion app catalog (PC, Android and TV, with search, categories, detail pages and downloads) sits next to Optimaxer's tuning tools: 199 tweaks with undo, services, startup items, a cleaner, DNS and network tools, and maintenance helpers. Written in Rust with eframe/egui.

Optimaxer started as a Windows PowerShell and WPF app. Its engine is ported to Rust here, so nothing in this build runs PowerShell except one call that creates a System Restore point.

## Build and run

```powershell
cargo build --release
.\target\release\orion-store-desktop.exe
```

`cargo test` runs 95 tests. Five more read the live system or the network and are ignored by default: `cargo test -- --ignored`. One of them downloads the latest release into a temp folder and checks the whole update path end to end.

The exe starts without administrator rights. The Store works as normal. The Optimaxer section can read everything, and shows a banner with a "Restart as administrator" button, because changing HKLM, services or tasks needs elevation.

## Design

A calm tool surface on a tonal ladder instead of lines, with indigo for selection and focus and lime for the one main action of each view. Three styles in Settings: Windows (the default: Windows 11 Fluent layering, 4 px controls, Segoe Fluent Icons, and your Windows light or dark mode and accent colour, followed live), Classic, and Material with the Material Design 3 palettes and pill-shaped controls, animated hover, selection and page transitions, and a reduce-motion switch that follows the Windows setting. Text contrast is checked by tests. The tokens, components and motion are written up in [DESIGN.md](DESIGN.md).

## Updates

The app checks GitHub for a newer release when it starts and shows a banner. Nothing is downloaded until you press "Update and restart". The zip is checked against the SHA-256 that GitHub publishes for it before anything is replaced, and a release without a checksum is refused. The running exe is renamed to `.old`, the new one takes its place, and the old file is removed on the next start. Settings has a switch for the startup check and a "Check now" button.

## The Store

Unchanged from the original client. It loads `config.json`, `apps.json` and `mirror.json` from GitHub, GitLab, Codeberg and JSDelivr in that order, falls back to a cached copy, then to the catalog embedded in the exe. Downloads run on worker threads with progress and cancel, and file names are sanitized before anything touches the disk. Downloads only start when you press a button.

## Optimaxer pages

Overview. Live gauges for processor, memory and the system drive, a 60 second history of CPU and memory, the processor, graphics, Windows edition, uptime and process count, and quick actions (Smart optimize, Free RAM, Scan for junk).

Tweaks. 199 of the 216 Optimaxer tweaks (performance, gaming, network, privacy, AI and Copilot removal, interface). Each tweak saves the original values before its first change, so it can be undone on its own, in any order. Presets are Auto-detect, Recommended, Gaming, Privacy, Remove AI and debloat, and Maximum.

Smart optimize and Auto-detect read the machine (laptop or desktop, SSD or HDD and NVMe, RAM, CPU threads, GPU, Windows build, board vendor) and select only the tweaks that fit and are not applied yet. A discrete GPU adds the gaming set, a laptop skips the always-max-power tweaks, and 16 GB+ RAM tweaks stay off smaller machines.

Services. About 220 services with Optimaxer's recommended Manual and Disabled lists. 57 core services are hidden and cannot be changed. A JSON backup is written before every change, and "Restore last backup" puts it back.

Startup. Run keys and Startup folders, switched on and off with the same StartupApproved flag Task Manager uses. Scheduled tasks that run at logon are not listed.

Cleaner. 32 targets: temp files, update and shader caches, browser and app caches, dumps, logs, event logs, Recycle Bin, component store and Windows.old. Scan first, then clean. Junctions and symlinks are never followed.

Network. DNS provider switch with DNS-over-HTTPS where the provider offers it, flush, stack reset, and a latency benchmark (TCP connect time to port 53).

Tools. Restore point, free RAM, TRIM, SFC, DISM, Windows Update reset, icon and Store cache reset, performance counters, registry backup, System Restore, and "Undo ALL Optimaxer tweaks".

## How the two halves connect

The Tools page suggests PC apps from the Orion catalog (Utility and Customization) and has buttons that open the Store on those categories. A finished download that is an installer gets a "Restore point + run" button in the Downloads list: Optimaxer creates a restore point first, and only then opens the file. You still decide inside the installer whether to continue.

Both halves share the theme, the `Settings` file, the data folder (`%LOCALAPPDATA%\OrionStore`) and the Downloads panel.

## Safety

Every action that changes the system opens a confirmation window first. For tweaks the window lists the exact registry values, services and tasks that will change (a read-only dry run), and offers a restore point. Advanced tweaks are marked in red. One job runs at a time, on a worker thread, so the window stays responsive.

Undo records use the same JSON layout as Optimaxer's `state.json`. On first start an existing `%ProgramData%\Optimaxer\state.json` is imported once into `%LOCALAPPDATA%\OrionStore\optimaxer\state.json`. The original file is never modified.

Nothing is downloaded or installed automatically, and Optimaxer never installs drivers. The vendor driver button only opens the maker's support page.

## What was not ported

17 tweaks stay in the PowerShell edition because they need Appx, MMAgent or similar cmdlets, or fetch data from the web: memory compression, app pre-launch and page combining, uninstalling OneDrive, folder type detection, uninstalling the Copilot and AI app packages, the Settings AI page, uninstalling Edge, the Razer and Logitech installer blocks, Opera startup, network adapter power saving, pausing Windows Update, the telemetry hosts block, Loudness Equalization, Game Bar popups, This PC folders and Store search suggestions. "Turn off Recall" applies its registry part and skips the DISM step.

Not in this build at all: Appx debloat, Windows optional features, WinGet app installs (the Orion catalog covers downloads), the hourly auto clean task, and scheduled-task startup entries.

## Data tables

`assets/optimaxer/tweaks.json` and `data.json` are exported from the PowerShell catalog by `tools/export-optimaxer.ps1`. The script is read only. Run it again after the PowerShell catalog changes:

```powershell
powershell -NoProfile -File tools\export-optimaxer.ps1
```

## Screenshots

`ORION_SHOTS=<folder>` makes the app walk through its pages, save each one as a PNG of its own window, and exit. It uses egui's viewport screenshot, so only the app window is captured.

## License

This folder is GPL-3.0 (see `LICENSE` here) because the Store client is derived from the Orion Store project, which is GPL-3.0. The PowerShell edition of Optimaxer in the rest of the repository stays MIT.

## Credits

The Store half is built on [Orion Store](https://github.com/RookieEnough/Orion-Store) by RookieEnough (GPL-3.0): the client code, the embedded offline catalog and the catalog format. Live catalog data comes from [Orion-Data](https://github.com/RookieEnough/Orion-Data). Thank you to everyone who works on it.

Optimaxer is MIT licensed, with tweak ideas from WinUtil and Win11Debloat (both MIT, see Optimaxer's `THIRD-PARTY-NOTICES.md`).
