<p align="center"><img src="assets/logo.png" width="96" alt="Optimaxer logo"></p>

# Optimaxer

**[Website](https://bliper2.github.io/optimaxer/)** · [Download](https://github.com/bliper2/optimaxer/releases/latest) · [Privacy](https://bliper2.github.io/optimaxer/privacy.html) · [Terms](https://bliper2.github.io/optimaxer/terms.html) · MIT licensed

Windows 10/11 performance, privacy, debloat and app-install toolkit. WPF GUI in Windows PowerShell 5.1, inspired by the layout of [Chris Titus Tech's WinUtil](https://github.com/ChrisTitusTech/winutil) (MIT), rebuilt with undo support, verification and a lot more tweaks.

## Get it
**One line** (installs to `%LOCALAPPDATA%\Optimaxer`, adds desktop and Start menu shortcuts, starts the app; run again to update):

```powershell
irm https://raw.githubusercontent.com/bliper2/optimaxer/main/install.ps1 | iex
```

Or download the zip from [Releases](https://github.com/bliper2/optimaxer/releases), extract it and double-click `Launch.bat` (asks for administrator rights).

## What it does
- **Install**: 260+ apps in 10 categories through WinGet (the WinUtil list plus more), with real app icons, "Show installed", upgrade-all, and a verification step after every install or uninstall.
- **Tweaks**: 170+ tweaks (performance, gaming, network, privacy, AI/Copilot removal, browser debloat, Windows Update control, interface) with presets. The original value of everything a tweak touches is saved, so each tweak, and "Undo ALL", restores your real prior settings. Optional restore point first.
- **Config**: export/import what you ticked on Install and Tweaks, create a desktop shortcut, and run unattended (see below).
- **Services**: ~240 services with recommended Manual/Disable lists, protected core services hidden, JSON backup before every change.
- **Startup**: registry, startup-folder **and scheduled-task** entries that run at logon/boot; enable or disable without uninstalling.
- **Cleaner**: scan, then clean temp files, update/shader/browser/dev-tool caches (npm, pip, NuGet, VS Code...), dumps, logs, event logs, font cache, Recycle Bin, component store and Windows.old.
- **Debloat**: curated Appx removal (Microsoft, OEM and game bloat) with safety levels.
- **Network**: DNS provider switch (+DoH), latency benchmark, stack reset; opt-in hosts-file telemetry block.
- **Features**: Hyper-V, WSL, Sandbox, .NET 3.5 and more.
- **Tools**: free RAM, TRIM/defrag, disk health, SFC, DISM, Windows Update reset, icon/Store cache, performance counters, registry backup, update check, System Restore.
- **Appearance**: themes (dark default, light and 8 more), optional animated wallpaper effects, layout and animation controls.

## Linux (Manjaro / Arch)
A separate, dependency-free Python edition lives in [`linux/`](linux/optimaxer.py). It works on Manjaro and other pacman-based distributions (EndeavourOS, Arch, Garuda...).

```bash
curl -fsSL https://raw.githubusercontent.com/bliper2/optimaxer/main/linux/install.sh | bash
optimaxer --dry-run     # explore the menu, nothing is changed
optimaxer               # real run (asks for sudo)
```

Menu sections: **Install apps** (96 apps from the official repos, the AUR through yay/paru, and Flatpak, each verified after install), **Tweaks** with presets and undo (zram, memory/swappiness, TRIM, I/O scheduler, journal limit, fast shutdown, earlyoom, BBR, pacman parallel downloads, paccache, hardening sysctls, optional ufw firewall...), **Cleaner** (pacman cache, orphans, journal, caches of browsers/pip/npm/Go/Cargo, Trash, unused Flatpak runtimes), **Services**, **Startup** (autostart entries), **Debloat packages**, **Network/DNS** (NetworkManager) and **Tools** (system info, update, Manjaro mirror ranking, failed units, .pacnew files, SMART health).

Non-interactive: `optimaxer tweaks list | apply --preset safe | undo --all`, `optimaxer apps install firefox brave-bin`, `optimaxer clean scan | run`, `optimaxer dns cloudflare | restore`, `optimaxer update`. Add `--dry-run` to see every command and file change first, `-y` to skip confirmations.

Safety: every change records the original state in `/var/lib/optimaxer/state.json` (files, config lines, service states, DNS), so `tweaks undo` restores it; edited config files are also copied to `/var/lib/optimaxer/backups`. Package installs by a tweak are kept on undo. Tested here with a simulated system (36 automated tests, `python -m unittest discover -s linux/tests`); I could not run it on a real Manjaro install, so start with `--dry-run`.

## Run unattended
```powershell
powershell -ExecutionPolicy Bypass -File Optimaxer.ps1 -Silent -Preset safe          # safe | gaming | privacy | deai | max
powershell -ExecutionPolicy Bypass -File Optimaxer.ps1 -Silent -Config my-setup.json # from Config > Export selection
```
Add `-NoRestorePoint` to skip the restore point. Exit code 0 = everything verified, 1 = something was not applied, 2 = bad arguments.

## Safety model
- Every tweak snapshots the prior registry values, service start types and task states (`%ProgramData%\Optimaxer\state.json`). Several tweaks sharing a setting still undo correctly in any order.
- Tweaks that remove keys (context-menu entries) export them to a `.reg` backup first. The hosts block lives between marker lines and the file is backed up.
- Anything risky or irreversible is tagged Moderate/Advanced, and opt-in tweaks never appear in presets.
- Logs: `%ProgramData%\Optimaxer\optimaxer.log`. Settings: `%ProgramData%\Optimaxer\settings.json`.

## Updates
Optimaxer checks GitHub Releases once per launch (Appearance > Updates, or Tools > Check for updates). A newer release shows an **Update to vX** button in the title bar. Updating downloads the zip, verifies its published SHA-256, backs up the current files to `%ProgramData%\Optimaxer\backups\app-<version>-<time>`, swaps them in after the app closes and restarts. A git checkout is never overwritten (use `git pull`). For a private fork set `OPTIMAXER_TOKEN` to a GitHub token with read access.

## Development
Linux edition tests: `python -m unittest discover -s linux/tests` (runs on any OS with a fake system).
`Optimaxer.ps1 -NoElevate -Screenshot <dir>` renders every page to PNG. `tools/make-logo.ps1` regenerates the logo files from `assets/logo.xaml`. Bump `VERSION` and include it in the release zip so the updater sees new releases.

## Credits
Tweak ideas draw on [Win11Debloat](https://github.com/Raphire/Win11Debloat) (MIT) and [WinUtil](https://github.com/ChrisTitusTech/winutil) (MIT), plus general knowledge of AtlasOS-style tuning (AtlasOS is GPL-3.0; no code copied). The telemetry hosts list is [WindowsSpyBlocker](https://github.com/crazy-max/WindowsSpyBlocker) (MIT), fetched at apply time.

App icons are fetched at runtime (never bundled) from open icon packs served by jsDelivr and cached in `%ProgramData%\Optimaxer\icons4`: [selfh.st icons](https://selfh.st/icons/) (CC BY 4.0, attribution: selfh.st), [Dashboard Icons](https://github.com/homarr-labs/dashboard-icons) (Apache-2.0) and [Simple Icons](https://simpleicons.org) (CC0). Apps without a pack icon fall back to their product page's favicon (Google favicon service, as WinUtil does). Logos remain the trademarks of their owners.
