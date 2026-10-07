# Optimaxer

Windows 10/11 performance, privacy and cleanup suite. WPF GUI in Windows PowerShell 5.1.
Inspired by the module layout of [Chris Titus Tech's WinUtil](https://github.com/ChrisTitusTech/winutil) (MIT), rebuilt with undo support and more tweaks.

## Run
Double-click `Launch.bat` (asks for admin). Nothing is installed.

## What it does
- **Optimize**: 100+ tweaks (performance, gaming, network, privacy, debloat, interface), presets (Recommended / Gaming / Privacy / Maximum). Original values are snapshotted per tweak (`%ProgramData%\Optimaxer\state.json`), so each tweak and "Undo ALL" restore real prior values. Optional restore point first.
- **Services**: ~240 services, recommended Manual/Disable lists, protected core services hidden, JSON backup before every change + restore.
- **Startup apps**: enable/disable registry and folder startup items (same mechanism as Task Manager).
- **Cleaner**: scan then clean temp, update cache, shader caches, browser caches, dumps, logs, Recycle Bin, component store.
- **Debloat apps**: curated Appx removal with safety levels.
- **Network**: DNS provider switch (+DoH), latency benchmark, stack reset.
- **Features**: Hyper-V, WSL, Sandbox, .NET 3.5, etc.
- **Tools**: free RAM, TRIM/defrag, disk health, SFC, DISM, reset Windows Update, icon cache, Store cache, System Restore.

## Look
Classic WinUtil-style layout: tab strip on top (Dashboard, Tweaks, Services, Startup, Cleaner, Debloat, Network, Features, Tools, Appearance), tweaks in three category columns with hover descriptions, and a log pane at the bottom. Dark by default; the Appearance tab adds a light theme, 8 more themes, optional animated wallpaper effects, gaps/rounding and a hyprland.conf-style preview. Saved to `%ProgramData%\Optimaxer\settings.json`.

Log: `%ProgramData%\Optimaxer\optimaxer.log`. Tweaks tagged opt-in (VBS off, OneDrive removal, search indexing off, reserved storage) never appear in presets.

Dev: `Optimaxer.ps1 -NoElevate -Screenshot <dir>` renders every page to PNG.

## Credits
Tweak ideas draw on [Win11Debloat](https://github.com/Raphire/Win11Debloat) (MIT) and [WinUtil](https://github.com/ChrisTitusTech/winutil) (MIT), plus general knowledge of AtlasOS-style tuning (AtlasOS is GPL-3.0; no code copied). All tweaks are implemented here with snapshot/undo.
