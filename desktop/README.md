<p align="center"><img src="../assets/logo.png" width="96" alt="Optimaxer logo"></p>

# Orion Store + Optimaxer for Windows

One Windows window, one exe. The Orion app catalog (PC, Android and TV, with search, categories, detail pages and downloads) sits next to Optimaxer's tuning tools: 200 tweaks with undo, services, startup items, a cleaner, DNS and network tools, and maintenance helpers. Written in Rust with eframe/egui.

Optimaxer started as a Windows PowerShell and WPF app. Its engine is ported to Rust here, so nothing in this build runs PowerShell except one call that creates a System Restore point.

## Build and run

```powershell
cargo build --release
.\target\release\orion-store-desktop.exe
```

`cargo test --locked` runs the offline regression suite. Live-system and network tests are ignored by default. One downloads the latest release into a temp folder and checks the update path; another empties this PC's memory caches, so inspect the ignored tests before running them. `cargo clippy --locked --all-targets -- -D warnings` checks all desktop targets.

The exe starts without administrator rights. The Store works as normal. The Optimaxer section can read everything, and shows a banner with a "Restart as administrator" button, because changing HKLM, services or tasks needs elevation.

## Design

A calm tool surface on a tonal ladder instead of lines, with indigo for selection and focus and lime for the one main action of each view. Settings keeps control shape separate from color: Windows, Classic and Material styles combine with 16 named palettes, including OLED, Nord, Solarized and custom accent. Comfortable or compact spacing and 90%, 100%, 110% or 125% interface scale persist and travel with versioned appearance presets. System/light/dark modes remain, with fixed dark/light palettes identifying their mode. A selected custom accent is saved and contrast-adjusted. Motion respects the Windows accessibility setting; hover, selection, page and modal transitions stay restrained. The Commands button or `Ctrl+K` opens a searchable palette for existing pages, Store search, Downloads and Settings. Text contrast is checked by tests. The tokens, components and motion are written up in [DESIGN.md](DESIGN.md). Current feature differences are in [the edition comparison](../docs/edition-comparison.md).

## Updates

The app checks GitHub for a newer release when it starts and shows a banner. Nothing is downloaded until you press "Update and restart". The zip is checked against the SHA-256 that GitHub publishes for it before anything is replaced, and a release without a checksum is refused. The running exe is renamed to `.old`, the new one takes its place, and the old file is removed on the next start. Settings has a switch for the startup check and a "Check now" button.

## The Store

Unchanged from the original client. It loads `config.json`, `apps.json` and `mirror.json` from GitHub, GitLab, Codeberg and JSDelivr in that order, falls back to a cached copy, then to the catalog embedded in the exe. Downloads run on worker threads with progress and cancel, and file names are sanitized before anything touches the disk. Downloads only start when you press a button.

## Optimaxer pages

Overview. Live gauges for processor, memory and the system drive, a 60 second history of CPU and memory, the processor, graphics, Windows edition, uptime and process count, quick actions, and the eight latest confirmed tweak batches. Expand a batch to inspect apply preview values, verification and undo availability. Undo entries identify saved-original counts; exact restored values remain in the existing undo store.

Activity. Searches a chronological timeline of up to 100 confirmed tweak apply/undo batches and up to 100 system-action output records. System-action records cover confirmed service, startup, cleaner, DNS, network reset, tools, installer launch, registry export and undo-all jobs. They preserve captured output only; they do not prove final system state or provide undo data. Read-only scans and benchmarks are excluded. Records live in separate, bounded `operations.json` under the app data directory.

Tweaks. 206 of the 217 Optimaxer tweaks (performance, gaming, network, privacy, AI and Copilot removal, interface). Each tweak writes its original values to disk before its first change, so it can be undone on its own, in any order. If a restore step fails, the record is kept and the log names the step. Presets are Auto-detect, Recommended, Gaming, Privacy, Remove AI and debloat, and Maximum; opt-in tweaks are in none of them. Tweaks that write the same setting (search box style, drive letters, taskbar grouping, All apps view, where Explorer opens) allow one choice per group.

Smart optimize and Auto-detect read the machine (laptop or desktop, SSD or HDD and NVMe, RAM, CPU threads, GPU, Windows build, board vendor) and select the recommended set plus the performance set where it fits: on a desktop, CPU power throttling off, no USB/PCIe link sleep and per-app timer requests honored; with a discrete GPU, hardware-accelerated GPU scheduling; Game Mode and Game DVR capture off. Advanced tweaks, the Ultimate plan, 100% minimum CPU state and boot-configuration changes are never selected automatically; each other row says why Smart optimize left it unticked (its risk, or a hardware fact such as "this PC is a laptop"). Retired tweaks (no measurable effect on Windows 11) are hidden unless you applied one earlier and want to undo it. If a requested restore point fails, nothing is applied until you confirm.

Services. About 220 services with Optimaxer's recommended Manual and Disabled lists. 57 core services are hidden and cannot be changed. A JSON backup is written before every change, and "Restore last backup" puts it back.

Startup. Run keys and Startup folders, switched on and off with the same StartupApproved flag Task Manager uses. Scheduled tasks that run at logon are not listed.

Cleaner. 32 targets: temp files, update and shader caches, browser and app caches, dumps, logs, event logs, Recycle Bin, component store and Windows.old. Scan first, then clean. Junctions and symlinks are never followed.

Network. DNS provider switch with DNS-over-HTTPS where the provider offers it, flush, stack reset, and a latency benchmark (TCP connect time to port 53).

Tools. Restore point, Free RAM (clears process and system working sets, the file cache, the modified and standby lists, identical pages and the registry cache), TRIM, SFC, DISM, Windows Update reset, icon and Store cache reset, performance counters, registry backup, System Restore, and "Undo ALL Optimaxer tweaks".

## How the two halves connect

The Tools page suggests PC apps from the Orion catalog (Utility and Customization) and has buttons that open the Store on those categories. A finished download that is an installer gets a "Restore point + run" button in the Downloads list: Optimaxer creates a restore point first, and only then opens the file. You still decide inside the installer whether to continue.

Both halves share the theme, the `Settings` file, the data folder (`%LOCALAPPDATA%\OrionStore`) and the Downloads panel.

## Safety

Every action that changes the system opens a confirmation window first. For tweaks the window lists the exact registry values, services and tasks that will change (a read-only dry run), and offers a restore point. Advanced tweaks are marked in red. One job runs at a time, on a worker thread, so the window stays responsive.

Undo records use the same JSON layout as Optimaxer's `state.json`. On first start an existing `%ProgramData%\Optimaxer\state.json` is imported once into `%LOCALAPPDATA%\OrionStore\optimaxer\state.json`. The original file is never modified.

Unreadable undo state blocks tweak changes until a valid backup is recovered. Failed undo operations retain their records for retry. Retrying a tweak preserves its first saved originals, and a failed command is reported even when other steps succeeded.

When a tweak batch changes a setting that needs a Windows restart, Optimaxer saves one reminder in its settings and shows it across pages. The reminder clears after Windows uptime resets. If the settings write fails, the Activity log and a toast report the error.

The Overview reads aggregate receive/send byte rates from active non-loopback Windows adapter counters and plots the last 60 samples. Virtual adapters may be included, so these readings describe adapter traffic and do not measure internet throughput. If Windows cannot return counters, the page reports them unavailable. The Processes page reads process CPU time, working-set memory and image paths on a background worker. CPU is an estimate over the three-second sample interval, normalized to total logical processor capacity. Windows-denied fields show as unavailable; the page never terminates processes. Publisher, GPU, temperature and disk-activity measurements are not included.

The Tweaks page can save up to 20 named selections locally and import or export versioned JSON profiles. Local profiles live at `%LOCALAPPDATA%\OrionStore\profiles.json`. Import selects only known, non-retired, non-opt-in tweaks that fit the current hardware; unavailable or incompatible IDs are skipped with a count. Import never applies changes. Applying a selected profile still requires administrator access, an exact preview and confirmation. Profiles contain tweak IDs only, not Windows settings or backup data. See the [profile format and import rules](../docs/profile-format.md).

Settings can import/export a versioned appearance preset containing theme mode, control style, palette, custom accent and reduced-motion preference. Import validates the file and updates only those appearance fields; it does not change Windows settings or other application preferences. The schema and validation are implemented in `src/appearance.rs`.

The verification scope and remaining integration checks are listed in [verification.md](../docs/verification.md).

Nothing is downloaded or installed automatically, and Optimaxer never installs drivers. The vendor driver button only opens the maker's support page.

## What was not ported

11 tweaks remain PowerShell-only because they require Appx package control, external downloads, ACL operations, or Windows APIs outside the Rust engine: uninstalling OneDrive, folder type detection, removing the Copilot and AI app packages, uninstalling Edge, blocking Razer and Logitech installers, Opera startup tasks, adapter power settings, telemetry hosts, Loudness Equalization and Store search permissions. "Turn off Recall" applies its registry part and skips the DISM step. Hiding the Settings AI page is supported with exact visibility-policy restoration.

Not in this build at all: Appx debloat, Windows optional features, WinGet app installs (the Orion catalog covers downloads), the hourly auto clean task, and scheduled-task startup entries.

## Data tables

`assets/optimaxer/tweaks.json` and `data.json` are exported from the PowerShell catalog by `tools/export-optimaxer.ps1`. The script is read only. Run it again after the PowerShell catalog changes:

```powershell
powershell -NoProfile -File tools\export-optimaxer.ps1
```

## Screenshots

`ORION_SHOTS=<folder>` makes the app walk through its pages, save each one as a PNG of its own window, and exit. It uses egui's viewport screenshot, so only the app window is captured. Set `ORION_SHOTS_SIDEBAR_COLLAPSED=true` to capture the Optimaxer icon rail, `ORION_SHOTS_RESTART_REQUIRED=true` to capture the restart reminder, or `ORION_SHOTS_PROFILE_OPEN=true` to expand Profiles on the Tweaks page. Screenshot mode never saves these overrides to settings.

## License

This folder is GPL-3.0 (see `LICENSE` here) because the Store client is derived from the Orion Store project, which is GPL-3.0. The PowerShell edition of Optimaxer in the rest of the repository stays MIT.

## Credits

The Store half is built on [Orion Store](https://github.com/RookieEnough/Orion-Store) by RookieEnough (GPL-3.0): the client code, the embedded offline catalog and the catalog format. Live catalog data comes from [Orion-Data](https://github.com/RookieEnough/Orion-Data). Thank you to everyone who works on it.

Profile open/save dialogs use [rfd 0.17.2](https://docs.rs/rfd/0.17.2/), MIT licensed. Its license is included in the repository's [third-party notices](../THIRD-PARTY-NOTICES.md).

Optimaxer is MIT licensed, with tweak ideas from WinUtil and Win11Debloat (both MIT, see Optimaxer's `THIRD-PARTY-NOTICES.md`).
