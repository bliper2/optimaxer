<p align="center"><img src="assets/logo.png" width="96" alt="Optimaxer logo"></p>

# Optimaxer

**[Website](https://bliper2.github.io/optimaxer/)** · [Download](https://github.com/bliper2/optimaxer/releases/latest) · [Privacy](https://bliper2.github.io/optimaxer/privacy.html) · [Terms](https://bliper2.github.io/optimaxer/terms.html) · MIT licensed

Windows 10/11 performance, privacy, debloat and app-install toolkit. WPF GUI in Windows PowerShell 5.1, inspired by the layout of [Chris Titus Tech's WinUtil](https://github.com/ChrisTitusTech/winutil) (MIT), rebuilt with undo support, verification and a lot more tweaks. Many settings, app lists and scripts are adapted from WinUtil and [Win11Debloat](https://github.com/Raphire/Win11Debloat) (MIT); see [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

## Screenshots

**Orion Store + Optimaxer** (the Rust desktop app, Windows 11 style with your system accent colour, dark and light):

| | |
|---|---|
| ![Overview with live gauges](docs/screenshots/desktop-overview.png) | ![Overview, light theme](docs/screenshots/desktop-overview-light.png) |
| ![The Orion app store](docs/screenshots/desktop-store.png) | ![Tweaks](docs/screenshots/desktop-tweaks.png) |

**Optimaxer for PowerShell** (WPF, Windows 10 and 11):

| | |
|---|---|
| ![Dashboard with live gauges](docs/screenshots/powershell-dashboard.png) | ![Tweaks](docs/screenshots/powershell-tweaks.png) |
| ![Apps](docs/screenshots/powershell-apps.png) | ![Cleaner](docs/screenshots/powershell-cleaner.png) |

## Get it
**One line** (installs to `%LOCALAPPDATA%\Optimaxer`, adds desktop and Start menu shortcuts, starts the app; run again to update):

```powershell
irm https://raw.githubusercontent.com/bliper2/optimaxer/main/install.ps1 | iex
```

Or download the zip from [Releases](https://github.com/bliper2/optimaxer/releases), extract it and double-click `Launch.bat` (asks for administrator rights).

## What it does
- **Install**: 260+ apps in 10 categories through WinGet (the WinUtil list plus more), with real app icons, "Show installed", upgrade-all, and a verification step after every install or uninstall.
- **Tweaks**: 213 tweaks (performance, gaming, network, privacy, AI/Copilot/Recall removal, a safe OneDrive uninstall, browser debloat, Windows Update control, interface) with presets. The 2026 taskbar, Start, Explorer, Game Bar and Store-search modules follow [WinUtil](https://github.com/ChrisTitusTech/winutil) and [Win11Debloat](https://github.com/Raphire/Win11Debloat). The original value of everything a tweak touches is saved, so each tweak, and "Undo ALL", restores your real prior settings. Optional restore point first.
- **Smart optimize**: detects laptop or desktop, SSD/NVMe or HDD, RAM size, GPU and Windows version, then selects only the tweaks that fit this PC and are not applied yet (Dashboard button, the Auto-detect chip on Tweaks, or `-Preset auto` unattended).
- **Config**: export/import what you ticked on Install and Tweaks, create a desktop shortcut, and run unattended (see below).
- **Services**: ~240 services with recommended Manual/Disable lists, protected core services hidden, JSON backup before every change.
- **Startup**: registry, startup-folder **and scheduled-task** entries that run at logon/boot; enable or disable without uninstalling.
- **Cleaner**: scan, then clean temp files, update/shader/browser/dev-tool caches (npm, pip, NuGet, VS Code...), dumps, logs, event logs, font cache, Recycle Bin, component store and Windows.old.
- **Debloat**: curated Appx removal (Microsoft, OEM and game bloat) with safety levels.
- **Network**: DNS provider switch (+DoH), latency benchmark, stack reset; opt-in hosts-file telemetry block.
- **Features**: Hyper-V, WSL, Sandbox, .NET 3.5 and more.
- **Auto clean**: one checkbox on the Dashboard schedules an hourly background task (runs as SYSTEM, even with Optimaxer closed) that trims process memory, purges the standby list and deletes temp files older than an hour. Turning it off removes the task.
- **Tools**: free RAM, TRIM/defrag, disk health, SFC, DISM, Windows Update reset, icon/Store cache, performance counters, registry backup, update check, System Restore.
- **Settings**: match-Windows, Dark, Light, OLED, Nord, Graphite, Dusk, Porcelain, Material or Material light, a Windows accent that follows your own colour plus nine more, live dashboard gauges, reduced-motion option and update-check control.

## Linux (Manjaro, Arch and Fedora)
A separate, dependency-free Python edition lives in [`linux/`](linux/optimaxer.py). It works on Manjaro and other pacman-based distributions (EndeavourOS, Arch, Garuda...) and on Fedora and other dnf-based ones (Fedora Workstation and spins, Nobara, RHEL, Rocky, AlmaLinux). It picks the package manager from `/etc/os-release`.

```bash
curl -fsSL https://raw.githubusercontent.com/bliper2/optimaxer/main/linux/install.sh | bash
optimaxer gui           # graphical window (needs Tk: sudo pacman -S tk, or sudo dnf install python3-tkinter); asks for your password
optimaxer --dry-run     # text menu, nothing is changed
optimaxer               # text menu, real run (asks for sudo)
```

The window (tkinter, no extra Python packages) has the same tabs as the Windows app: Home, Install, Tweaks with presets, Cleaner, Services, Startup, Debloat, Network, Tools, Config (export/import a selection, add an application-menu launcher) and Appearance (9 themes, including Material dark and light), plus a Dry run switch and a live log. `install.sh` adds Optimaxer to your application menu.

Sections: **Install apps** (97 apps from the official repos, the AUR through yay/paru, and Flatpak, each verified after install), **Tweaks** with presets and undo (zram, memory/swappiness, TRIM, I/O scheduler, journal limit, fast shutdown, earlyoom, BBR, pacman parallel downloads, paccache, hardening sysctls, optional ufw firewall, **browser debloat** for Firefox, Chromium, Chrome, Brave and Edge...), **Cleaner** (pacman cache, orphans, journal, caches of browsers/pip/npm/Go/Cargo, Trash, unused Flatpak runtimes), **Services**, **Startup** (autostart entries), **Debloat packages**, **Network/DNS** (NetworkManager) and **Tools** (system info, update, Manjaro mirror ranking, failed units, .pacnew files, SMART health).

On Fedora the same sections work through dnf: apps come from the Fedora repositories or Flathub (the Flathub remote is added when it is missing; AUR-only apps and apps that need a third-party repository are not offered), there is a dnf tweak (parallel downloads and fastest mirror), the cleaner handles the dnf cache and unneeded packages, the update tool runs `dnf upgrade --refresh`, leftover-config search looks for `.rpmnew` and `.rpmsave` files, and the Arch-only tweaks (pacman, ufw; Fedora already runs firewalld) are hidden.

Non-interactive: `optimaxer tweaks list | apply --preset safe | undo --all`, `optimaxer apps install firefox brave-bin`, `optimaxer clean scan | run`, `optimaxer dns cloudflare | restore`, `optimaxer config apply my-setup.json`, `optimaxer launcher`, `optimaxer update`. Add `--dry-run` to see every command and file change first, `-y` to skip confirmations.

Safety: every change records the original state in `/var/lib/optimaxer/state.json` (files, config lines, service states, DNS), so `tweaks undo` restores it; edited config files are also copied to `/var/lib/optimaxer/backups`. Package installs by a tweak are kept on undo. Tested here with a simulated system (62 automated tests covering Arch and Fedora, including the window, `python -m unittest discover -s linux/tests`); I could not run it on a real Manjaro install, so start with `--dry-run`.

## Desktop app (Rust)
[`desktop/`](desktop/) holds a single-exe Windows app that puts the [Orion Store](https://github.com/RookieEnough/Orion-Store) catalog and Optimaxer in one window (Rust, eframe/egui). The Optimaxer side is a port of the engine to Rust: 199 of the 216 tweaks with the same undo records, plus services, startup, cleaner, DNS and network tools, and maintenance helpers. Smart optimize reads your hardware and selects only the tweaks that fit. The window follows Windows 11's Fluent look and your accent colour, and the Overview page shows live CPU, memory and disk gauges. It checks for a newer release at startup and updates in one click, with the download checked against its published SHA-256, and Settings has a Material design option. Every change opens a confirmation window that lists what will change. The 17 tweaks that need PowerShell cmdlets stay in this edition. Download `OrionStore-Optimaxer-desktop-v*-win-x64.zip` from [Releases](https://github.com/bliper2/optimaxer/releases/latest), or build it with `cargo build --release` in `desktop/`. The desktop folder is GPL-3.0 (it derives from Orion Store); everything else here stays MIT. See [desktop/README.md](desktop/README.md).

## Run unattended
```powershell
powershell -ExecutionPolicy Bypass -File Optimaxer.ps1 -Silent -Preset safe          # auto | safe | gaming | privacy | deai | max
powershell -ExecutionPolicy Bypass -File Optimaxer.ps1 -Silent -Config my-setup.json # from Config > Export selection
```
Add `-NoRestorePoint` to skip the restore point. Exit code 0 = everything verified, 1 = something was not applied, 2 = bad arguments.

## Safety model
- Every tweak snapshots the prior registry values, service start types and task states (`%ProgramData%\Optimaxer\state.json`). Several tweaks sharing a setting still undo correctly in any order.
- Tweaks that remove keys (context-menu entries) export them to a `.reg` backup first. The hosts block lives between marker lines and the file is backed up.
- Anything risky or irreversible is tagged Moderate/Advanced, and opt-in tweaks never appear in presets.
- Logs: `%ProgramData%\Optimaxer\optimaxer.log`. Settings: `%ProgramData%\Optimaxer\settings.json`.

## Updates
Optimaxer checks GitHub Releases once per launch (Settings > Updates, or Tools > Check for updates). A newer release shows an **Update to vX** button in the title bar. Updating downloads the zip, verifies its published SHA-256, backs up the current files to `%ProgramData%\Optimaxer\backups\app-<version>-<time>`, swaps them in after the app closes and restarts. A git checkout is never overwritten (use `git pull`). For a private fork set `OPTIMAXER_TOKEN` to a GitHub token with read access.

## Development
Desktop app tests: `cargo test` in `desktop/`.
Linux edition tests: `python -m unittest discover -s linux/tests` (runs on any OS with a fake system).
`Optimaxer.ps1 -NoElevate -Screenshot <dir>` renders every page to PNG. `tools/make-logo.ps1` regenerates the logo files from `assets/logo.xaml`. Bump `VERSION` and include it in the release zip so the updater sees new releases.

## Credits
The desktop app's Store client and app catalog come from [Orion Store](https://github.com/RookieEnough/Orion-Store) by RookieEnough (GPL-3.0), with catalog data from [Orion-Data](https://github.com/RookieEnough/Orion-Data). Thank you to everyone who works on it.

Tweak ideas draw on [Win11Debloat](https://github.com/Raphire/Win11Debloat) (MIT) and [WinUtil](https://github.com/ChrisTitusTech/winutil) (MIT), plus general knowledge of AtlasOS-style tuning (AtlasOS is GPL-3.0; no code copied). The telemetry hosts list is [WindowsSpyBlocker](https://github.com/crazy-max/WindowsSpyBlocker) (MIT), fetched at apply time.

App icons are fetched at runtime (never bundled) from open icon packs served by jsDelivr and cached in `%ProgramData%\Optimaxer\icons4`: [selfh.st icons](https://selfh.st/icons/) (CC BY 4.0, attribution: selfh.st), [Dashboard Icons](https://github.com/homarr-labs/dashboard-icons) (Apache-2.0) and [Simple Icons](https://simpleicons.org) (CC0). Apps without a pack icon fall back to their product page's favicon (Google favicon service, as WinUtil does). Logos remain the trademarks of their owners.
