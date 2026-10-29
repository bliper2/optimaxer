# Edition comparison

Snapshot of the repository at v2.15.4 (`2672279`). This describes implemented scope, not a parity promise. The Rust desktop app is the primary target for the next UI phase; the PowerShell and Linux editions remain independent products.

| Area | PowerShell / WPF (Windows) | Python / Tkinter (Linux) | Rust / egui (Windows) |
| --- | --- | --- | --- |
| Delivery | Windows app and unattended CLI | Arch-family and Fedora-family GUI and CLI | Windows desktop app, Orion Store and Optimaxer in one window |
| Tweaks | 209 active, 8 retired; presets and exact undo | Linux-specific catalog, presets, dry run and undo | 206 of 217 catalog entries; 11 PowerShell-only entries omitted; presets, dry run/preview and exact undo |
| Services | About 240, protected core rows hidden, backup and restore | systemd service list, enable/disable and restore | About 220, protected core rows hidden, JSON backup and restore |
| Startup | Registry, folders and scheduled tasks | User autostart entries | Registry Run keys and Startup folders; scheduled tasks omitted |
| Cleaner | Scan-first Windows targets, including component store and Windows.old | Package-manager caches, user caches, logs and Trash | 32 Windows targets; scan first; links are not followed |
| Packages | WinGet install, upgrade, remove and verify | pacman, AUR helper, dnf and Flatpak flows | Orion catalog downloads; no package-manager install/remove workflow |
| Network | DNS/DoH, benchmark, reset and opt-in hosts block | NetworkManager DNS and active connections | DNS/DoH, benchmark and reset |
| Tools | Windows maintenance, boot cost, game profiles, wake sources, search exclusions and PresentMon measurement | System updates, mirrors, failed units, `.pacnew`/`.rpmnew`, SMART health | Restore point, memory, TRIM, SFC/DISM, cache resets, registry backup, network and maintenance tools |
| Appearance | System/dark/light and several named palettes; motion setting | Nine Tk themes and dry-run control | System/light/dark mode, three shape styles, 16 named palettes plus saved custom accent, reduced motion |
| Data portability | Tweak/app selection export and import | Tweak/package selection export and import | No equivalent profile/config import/export workflow |

## Rust desktop baseline and gaps

The Rust app has seven Optimaxer pages: Overview, Tweaks, Services, Startup, Cleaner, Network and Tools. The Overview samples real CPU, physical-memory, system-drive, uptime and process readings once per second. It does not currently sample network throughput, GPU load, temperatures or per-process resource use; the UI must not imply those readings exist.

Settings expose system/light/dark mode, Windows/Classic/Material control styling, 16 named color palettes, saved custom accent, reduced motion, update checks, download location and restore-point preference. Palette choice remains separate from control shape; custom accent text and action fills are contrast-adjusted. Density, font and corner-radius controls, theme import/export and full custom surface editing are not implemented. The navigation rail is fixed-width. There is no separate Performance Center, Gaming Hub, operation timeline, profiles page, notification center or system snapshot browser. Existing exact-undo records, activity log, service backups and confirmation previews remain the source of truth for their respective operations.

The command palette is the first addition from this audit: `Ctrl+K` and the visible Commands button expose existing pages, Store search, Downloads and Settings. It does not claim absent features or trigger system changes.

## Next implementation constraints

- Keep controls attached only to implemented operations. New destinations require a complete read/change/restore path before they appear in navigation.
- Add measurements only when a real platform probe, error state and units are defined.
- Extend theme choices through validated token sets and contrast checks; keep shape style and color palette separate.
- Keep PowerShell-only Windows operations out of Linux and Rust until each port has a tested native implementation.
- Update this comparison when source behavior changes; use the edition READMEs for operational instructions and [verification.md](verification.md) for test evidence and remaining machine-level checks.
