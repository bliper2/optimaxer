# Application verification

Optimaxer has three implementations. A passing suite verifies its covered behavior; it does not prove every catalog operation on every supported operating system.

## Automated checks

| Edition | Command | Coverage |
| --- | --- | --- |
| PowerShell | `powershell -NoProfile -ExecutionPolicy Bypass -File tests/Optimaxer.Tests.ps1` | Catalog and preset rules, scratch-registry apply/undo, failure reporting, backup refusal, restoration, cleaner boundaries and selected tools. System-changing commands are stubbed. |
| Rust desktop | `cargo test --locked` in `desktop/` | Fake-system engine tests, saved originals and retries, service/task restoration, cleaner boundaries, update staging, local HTTP download failures, catalog processing, UI batch behavior, command-palette filtering, activity search and mixed timeline ordering, theme contrast across palettes/styles, density spacing and scale factors, settings migration, versioned appearance preset validation/field scope including version 1 migration, bounded activity-history persistence/failure handling, adapter-rate delta/reset/unit handling, bounded disk telemetry history and window reset behavior, system snapshot serialization/cap/corrupt-store preservation and recent-capture comparisons that exclude changing memory use, used disk space and uptime, profile validation and compatibility filtering, read-only process scans/CPU calculation, and fixed read-only SFC/DISM invocation with bounded output and exit-code preservation. Some updater tests use real Windows tools against temporary files. |
| Rust desktop | `cargo clippy --locked --all-targets -- -D warnings` | Static checks for production and test targets. |
| Rust desktop | `cargo build --release --locked` | Release executable build. |
| Linux | `python -B -m unittest discover -s linux/tests` | Simulated Arch/Fedora packages, files, systemd and NetworkManager; state failures, exact service restoration, dry run, startup flags, cleanup and updater validation. Tk tests exercise real widgets when a display is available. |
| Linux installer | `bash -n linux/install.sh` | Shell syntax. |

GitHub Actions runs Windows checks and Linux tests on both Windows and Ubuntu. The Ubuntu job supplies a virtual display for Tk. No CI job applies optimization presets to its host.

## Desktop smoke checks

The built-in screenshot modes render each page without pressing system-changing controls. The latest Windows debug screenshot walk contains fifteen Rust steps. Activity was visually checked at 1180 × 760 (`docs/screenshots/desktop-activity.png`), Gaming at 1440 × 900 (`docs/screenshots/desktop-gaming.png`), Diagnostics at 1440 × 900 (`docs/screenshots/desktop-diagnostics.png`), and System Snapshot at 1440 × 900 (`docs/screenshots/desktop-system-snapshot.png`). The snapshot capture shows the real empty state; creating one was separately exercised by the store tests, not during screenshot mode. The earlier maintenance audit rendered all twelve PowerShell pages. Rust checks include dark mode at 1180 × 760, light mode at 760 × 520, compact density at 125% scale in a 760 × 520 window, and the Performance page at 1440 × 900. The compact Performance capture (`docs/screenshots/desktop-performance-compact.png`) verifies that metric and chart groups stack without horizontal clipping and remain vertically scrollable. The compact run also exposed and verified wrapped service/startup/cleaner controls and a stacked Tweaks action. Live Windows adapter receive/send counters and aggregate PhysicalDisk throughput were shown in the latest Performance capture (`docs/screenshots/desktop-performance.png`); PowerShell `Get-Counter` independently returned the PhysicalDisk `Disk Bytes/sec` counter on the verification host. Navigation and page content scroll in compact windows, and the store search moves below the title controls when needed. Applying or undoing changes was not exercised during screenshot verification. The SFC/DISM commands themselves are not run during UI screenshot verification.

Rendering confirms that pages open and produce images. It does not verify every click, keyboard path, display scale or assistive technology.

The v2.15.5 Windows release build was also opened interactively at 1180 × 760: Commands opened the palette, searching `Tweaks` returned one result, and Enter navigated to the Tweaks page. This is a focused smoke check, not full keyboard-only or accessibility testing.

The v2.15.6 Windows release build was opened at 1180 × 760. Settings content scrolled to the lower sections, the Arctic palette changed the live interface, and the selected palette remained after restarting the app. The original settings file was restored after the check. This does not cover every palette or display scale.

A v2.15.7 build was rendered at 1180 × 760 with the Optimaxer rail collapsed. The overview screenshot was visually checked, and the Network page accessibility tree exposed full navigation names. Settings serialization tests cover the saved collapse choice and the default for older settings files; toggle persistence across an ordinary app restart still needs a manual check.

The Windows debug screenshot run generated all nine app-page images with `ORION_SHOTS_RESTART_REQUIRED=true`; the overview banner was visually checked at 1180 × 760. Screenshot mode keeps settings read-only. Running the release executable directly exited without producing images, so release-mode screenshot capture remains unverified. Unit tests cover reminder serialization, older-settings defaults, clearing after uptime resets and atomic-write failure cleanup. A real Windows reboot round trip has not been run.

The profile-enabled Tweaks page screenshot was generated at 1180 × 760 with `ORION_SHOTS_PROFILE_OPEN=true`. Profile tests cover version and identifier validation, corrupt-store preservation, cross-PC applicability filtering, conflicting tweak rejection, profile differences and replacement of existing profile/export files. Native file-dialog export was exercised on Windows and produced valid JSON. Interactive import selection, overwrite confirmation and operation on another Windows profile still need manual checks.

Per-game graphics preference tests use the fake registry to cover exact original-value restoration, preservation of other directives in the Windows preference string, Windows-default removal, preview drift, access denial, external edits, failed persistence and corrupt profile data. Portable preset tests verify that only the preference is exported, never the executable path or undo state. These are engine tests; the executable picker, preset dialogs, real Windows registry write/restore, and display-driver behavior still need manual verification on a disposable Windows account.

Appearance preset tests cover round-trip, format/mode/accent validation, limited field application and replacement of an existing file. Native appearance import/export controls compile and use the validated file format; interactive dialogs and persistence across an app restart still need manual checks.

The v2.15.7 fake-system tests verify apply/undo history values, status and undo eligibility. The history store tests its 100-entry bound and refusal to overwrite corrupt data. No live system tweaks were run for this check.

Process Explorer tests cover CPU normalization, invalid intervals, counter rollback and a read-only scan of the test process's image path and working set without elevation. Activity tests cover searches across dates, actions, names, IDs and captured details. Protected-process behavior and refresh cost still need manual checks on Windows 10 and 11.

## Failure and restoration rules

- Invalid undo state blocks mutations instead of being replaced with an empty store.
- Originals are saved before changes. Retrying keeps the first saved values; newly discovered declarative targets need their own saved originals before mutation.
- Failed restore steps keep their records for another attempt. Missing originals are reported rather than replaced by guessed defaults.
- Command failures are reported even if other steps succeeded. Cleanup totals count measured or successfully deleted data.
- Release updates require a published SHA-256 checksum. Staging failures leave the working application available.
- Package installation by a Linux tweak is retained on undo. Deleted caches, removed app data and cleared logs are not restored by tweak undo.

## Remaining integration checks

Use disposable machines or snapshots for operations that change the operating system:

- Windows 10 and 11: apply and undo registry, services and scheduled tasks with normal, elevated and denied access. Compare with the captured originals.
- Boot, power, filesystem, reserved-storage and hibernation changes: reboot and verify effective state. Hibernation undo preserves enabled/disabled state; it does not recreate the former hibernation file contents or mode.
- Arch and Fedora: real package managers, systemd enabled/active combinations, NetworkManager DNS changes and restoration, and desktop launcher registration.
- Updates: running-executable replacement, destination permissions, interruption, and rollback on a locked destination. Check both application restart and the retained backup.
- UI: keyboard-only use, display scaling, long localized names, and interactive controls at minimum window size.

The Rust edition deliberately omits the PowerShell features listed in its [README](../desktop/README.md#what-was-not-ported). This audit does not add those features or claim live round-trip validation for every tweak.
