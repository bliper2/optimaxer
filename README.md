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
Tiling-WM style UI: gapped rounded tiles, waybar-style bar with workspace pills (1-9 modules, 0 = appearance), rotating gradient border on the focused tile (focus follows mouse). Workspace `0` (`~/rice`): 8 themes (catppuccin, tokyo night, gruvbox, rose pine, nord, dracula, everforest, hyprland default), 4 animated wallpaper effects (aurora, starfield, rain, embers), gaps / rounding / border size, animation speed, six one-click rices, and a live equivalent `hyprland.conf` snippet. Saved to `%ProgramData%\Optimaxer\settings.json`. For the lowest CPU use pick effect `none` and animations `off`.

Log: `%ProgramData%\Optimaxer\optimaxer.log`. Tweaks tagged opt-in (VBS off, OneDrive removal, search indexing off, reserved storage) never appear in presets.

Dev: `Optimaxer.ps1 -NoElevate -Screenshot <dir>` renders every page to PNG.
