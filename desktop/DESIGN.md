# Design notes

How the desktop app looks and moves, and where each decision lives in the code. Tokens are in `src/theme.rs`, components in `src/kit.rs`.

## Direction

A calm tool surface. Surfaces step in tone instead of being drawn with lines, one accent marks selection and focus, and one colour marks the main action of a view. Nothing glows, nothing has a shadow at rest, and motion only answers what you just did.

The brand colours stay: indigo `#7478F8` for selection, focus and navigation, lime `#A3E635` for the one primary action per view (Download, Smart optimize, Apply, Clean).

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

## Type

Segoe UI, with Segoe UI Semibold for titles, buttons and emphasis. Scale in px: caption 12, small 13, body 14, heading 16, title 22, display 28. Labels are sentence case; nothing is set in tracked capitals.

## Space and shape

Spacing scale 4, 8, 12, 16, 24, 32. Radius scale 6 (chips), 8 (controls), 12 (cards), 16 (windows). Controls are at least 34 px tall.

## Components

Primary button: lime fill, dark text, one per view. Secondary: raised surface with a border that turns brand on hover. Ghost: no fill until hover. Chip: tinted pill in one of five tones (neutral, brand, good, warn, bad). Card: surface1 with a hairline border. Navigation item: hover tint, brand-tinted selection with an indicator that grows in, optional count. Segmented control: the selected thumb slides between options. Row: animated hover tint, whole row is the click target. Tile: title, wrapped description, optional note.

States: every control has rest, hover, pressed, disabled and keyboard focus. Focus draws a 2 px ring in the accent colour. Disabled controls drop to the subtle text colour on surface2.

## Motion

Three lengths: fast 90 ms (hover, press), base 160 ms (selection, toggles), slow 260 ms (page enter). Everything eases out. Pages fade in with a 10 px rise when you switch view. The Store/Optimaxer thumb slides, toasts fade and rise in and fade out, and loading content pulses as skeleton rows.

Reduced motion makes every duration zero and stops the pulse. It follows the Windows setting (Accessibility, Visual effects, Animation effects) and can be switched in Settings.

## Accessibility

Controls report a role and label to the accessibility tree. All interactive rows, cards and buttons take keyboard focus and activate with Enter or Space. Contrast is enforced by tests. Nothing relies on colour alone: applied tweaks carry an "Applied" chip, risk carries a word.

## Writing

Buttons say what happens and use the same words through a flow: "Apply 12 tweaks" opens a window titled "Apply tweaks" and ends in a log line "Finished". Empty and not-yet-loaded views say what is missing and what to do. Errors name the cause and the fix. No exclamation marks, no filler.

## Screenshots

`ORION_SHOTS=<folder>` writes a PNG of each page from the app's own window. `ORION_SHOTS_THEME=light` renders the light theme. This run never writes to your saved settings.
