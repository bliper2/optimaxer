# Design notes

How the desktop app looks and moves, and where each decision lives in the code. Tokens are in `src/theme.rs`, components in `src/kit.rs`.

## Direction

A calm tool surface. Surfaces step in tone instead of being drawn with lines, one accent marks selection and focus, and one colour marks the main action of a view. Nothing glows, nothing has a shadow at rest, and motion only answers what you just did.

The System palette keeps Optimaxer's indigo `#7478F8` for selection, focus and navigation, with lime `#A3E635` for the one primary action per view. Named palettes use their own restrained accent for selection and primary actions.

The WinGet page shares these tokens and row components. It separates the Orion download catalog from software installation, labels exported package matches as incomplete inventory, and keeps package-manager output in a visible console. Confirmation shows the exact ID-scoped command before launch.

At the 760 px minimum window width, WinGet moves its toolbar below the page introduction and wraps catalog rows so icons, package details and actions stay inside the content area.

## Colour

| Token | Dark | Light | Use |
|---|---|---|---|
| canvas | `#0F1015` | `#F6F5FC` | content area |
| rail | `#0B0C10` | `#EEEDF8` | top bar, sidebar, status bar |
| surface1 | `#171821` | `#FFFFFF` | cards, panels |
| surface2 | `#1F202B` | `#F0EFF9` | inputs, hover |
| surface3 | `#2A2B3A` | `#E4E3F2` | pressed, selected thumb |
| hairline | `#2B2C39` | `#DDDCEC` | card borders only |
| text | `#ECEDF3` | `#1B1C26` | body and titles |
| muted | `#A9ACBD` | `#52556C` | secondary text |
| subtle | `#8A8DA2` | `#666980` | captions, counts |
| brand text | `#A3A6FF` | `#464BCC` | links, group titles |

Every text colour is tested against every surface it can sit on (`cargo test`, module `theme`): body text at 7:1 or better, muted and subtle at 4.5:1, status tones at 4.5:1 on their tinted chips, the focus ring at 3:1.

Settings offers 23 named palettes: Midnight, OLED, Graphite, Nebula, Nord, Arctic, Fluent dark/light, Material dark/light, Emerald, Crimson, Solarized, Porcelain, Cyberpunk, Obsidian, Spectra, Ember, Tokyo Night, Rose Quartz, Aurora, Retro Terminal and Custom theme. System remains the default and follows Windows when using the Windows style. Other palettes adapt to the selected light or dark mode, except the four explicitly labeled dark/light palettes, which lock their mode. Palette colors do not change control shape. Custom theme lets users edit the accent and six surface colors in both dark and light modes. Text colors adjust to preserve contrast; imported color sets that fail contrast checks are rejected. Density, font and corner radius remain separate controls. Versioned appearance presets export/import theme mode, control style, palette, custom accent and surfaces, density, scale, corner radius and reduced-motion preference, without exporting other application or system settings. Contrast tests cover each palette with all three control styles.

## Windows style (default)

Follows Windows 11: the Windows light or dark mode and your accent colour (read from personalization, re-read every couple of seconds, with a fallback to the stock blue when the registry holds none) while System palette is selected; the content area one tonal step above the rail around it, 4 px controls and 8 px cards, and Segoe Fluent Icons in the sidebar. The accent is adjusted until text in it reads at 4.5:1 on every surface, and button text switches between white and near-black (or the fill darkens) for the same reason. The contrast tests run against eight sample Windows accents in both themes.

## Material option

Settings has a Material design switch. It swaps in the Material Design 3 baseline palettes (dark `#141218` canvas with the `#D0BCFF` primary, light `#FEF7FF` canvas with the `#6750A4` primary), makes buttons and navigation items fully rounded, and uses the primary colour for the main action instead of lime. The hover state lifts by a light 8% state layer. The same contrast tests run on all four palettes.

## Type

Segoe UI, with Segoe UI Semibold for titles, buttons and emphasis. Scale in px: caption 12, small 13, body 14, heading 16, title 22, display 28. Labels are sentence case; nothing is set in tracked capitals.

## Space and shape

Spacing scale 4, 8, 12, 16, 24, 32. Radius scale 6 (chips), 8 (controls), 12 (cards), 16 (windows). Controls are at least 34 px tall.

## Components

Primary button: lime fill, dark text, one per view. Secondary: raised surface with a border that turns brand on hover. Ghost: no fill until hover. Chip: tinted pill in one of five tones (neutral, brand, good, warn, bad). Card: surface1 with a hairline border. Navigation item: hover tint, brand-tinted selection with an indicator that grows in, optional count. Segmented control: the selected thumb slides between options. Row: animated hover tint, whole row is the click target. Tile: title, wrapped description, optional note.

States: every control has rest, hover, pressed, disabled and keyboard focus. Focus draws a 2 px ring in the accent colour. Disabled controls drop to the subtle text colour on surface2.

The Optimaxer sidebar can collapse to a 64 px icon rail. Its expanded destinations are grouped under Workspace, Optimization and System. Labels stay available as tooltips and accessibility names; the detected PC summary returns when expanded. The choice persists in application settings. Store navigation remains expanded because its platform and category entries use text labels and counts.

Smart Optimize is a Workspace page. It shows the hardware facts and inactive recommendations produced by the existing hardware-aware plan, then lists exclusion reasons. Refresh reads current tweak state on a worker. Review selects only the plan's existing IDs in Tweaks; preview and confirmation remain the only path to apply changes.

Startup rows retain their checkbox behavior for batch enable/disable. A separate Details action opens the original launch command and, only when an executable path is explicit, its location and embedded CompanyName. The panel labels this as file metadata rather than verified publisher identity; links and ambiguous launch commands are reported as unresolved.

Service changes query each selected service's direct dependencies and dependents before opening the existing confirmation. The dialog identifies load-order groups and flags dependents when disabling; it shows a lookup error and offers no proceed action if Windows cannot return the information.

The Appearance settings palette picker is a selectable gallery rather than a text-only menu. Each choice shows its live accent, success and warning colors; selecting a palette immediately reapplies the app theme. The old palette is restored if the settings file cannot be saved.

The System Snapshot page uses a compact, collapsible history because each capture is a point-in-time inventory rather than a live dashboard. It labels unavailable readings explicitly and states that local snapshots cannot restore Windows.

The Store Downloads panel separates active session transfers from recent completed files. Recent files come from the local bounded history, with Open and Show in folder actions; the UI does not infer installation state from a download.

## Background work and cancellation

Workers receive a cancellation token only for read-only Cleaner size scans. The scan checks it during wildcard expansion and directory traversal; a cancelled scan discards all partial sizes and selections. Mutating jobs do not receive a cancellation token, because stopping them after a system change or file deletion could leave partial state without a reliable rollback path.

## Motion

Three lengths: fast 90 ms (hover, press), base 160 ms (selection, toggles), slow 260 ms (page enter). Everything eases out. Pages fade in with a 10 px rise when you switch view. The Store/Optimaxer thumb slides, toasts fade and rise in and fade out, and loading content pulses as skeleton rows. The activity dock uses the same global animation time when it opens or closes, including the reduced-motion setting.

Reduced motion makes every duration zero and stops the pulse. It follows the Windows setting (Accessibility, Visual effects, Animation effects) and can be switched in Settings. The Optimaxer page transition runs once per page change; entering the section and changing a page use the same short fade and rise.

## Accessibility

Controls report a role and label to the accessibility tree. All interactive rows, cards and buttons take keyboard focus and activate with Enter or Space. Contrast is enforced by tests. Nothing relies on colour alone: applied tweaks carry an "Applied" chip, risk carries a word.

## Writing

Buttons say what happens and use the same words through a flow: "Apply 12 tweaks" opens a window titled "Apply tweaks" and ends in a log line "Finished". Empty and not-yet-loaded views say what is missing and what to do. Errors name the cause and the fix. No exclamation marks, no filler.

## Screenshots

`ORION_SHOTS=<folder>` writes a PNG of each page from the app's own window. `ORION_SHOTS_THEME=light` renders the light theme. This run never writes to your saved settings.

## Black Box and Tweaks detail levels

Black Box is off until the user turns it on. It keeps the newest 8,640 readings (CPU, memory, disk, network, GPU) in `optimaxerlackbox.json`, records no process, window or file names, and runs only while the app is open. Charts keep each column's peak so short spikes stay visible, and readings Windows does not provide are gaps, never zeros. Findings are limited to CPU or memory at or above 90% for 60 seconds. The System log lookup shows provider and event ID only.

Tweaks has three detail levels, saved in settings. Beginner lists only Safe, auto-pickable tweaks that fit this PC and hides filters, profiles and technical text. Enthusiast is the full catalog with risk, restart and undo chips. Expert opens every tweak's exact settings, scope, undo behaviour, derived trade-offs and the originals Optimaxer saved. Undo is labelled Reversible, Conditionally reversible or Not reversible from the catalog data; Optimaxer does not claim a speed effect for any tweak.

## Bottleneck Detective

A guided 30-second measurement, or an analysis of the last hour of Black Box readings. Each finding is marked Observed (the readings show it) or Hypothesis (a plausible explanation they cannot confirm), carries its numbers, and offers a next step that opens the relevant page. Processor and memory are reported when at or above 90% for at least 20% of readings; graphics is only ever a hypothesis. Areas that were measured and fine are listed with their numbers, and areas that cannot be measured (temperature, network quality, disk queue and latency) are listed as not assessed. There is no score.

## Startup X-Ray

Reads Windows' Diagnostics-Performance log (administrator only): boot durations (event 100) and the applications, services and drivers Windows itself measured as slowing boots (events 101 to 103). It shows a bar chart of up to 30 boots, a trend that is stated only with four or more boots and only when the change exceeds both a quarter of the earlier median and half their spread, items flagged on more than one boot, a two-boot comparison, a service dependency view, a System-log lookup around a boot, and boot medians before and after each startup change made in Optimaxer. Windows-protected services are marked and never offered for disabling here. Per-app impact is Windows' measurement, not an Optimaxer estimate.
