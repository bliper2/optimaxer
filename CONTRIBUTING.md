# Contributing

Bug reports, tweak ideas and pull requests are welcome.

## Report a bug

Open an [issue](https://github.com/bliper2/optimaxer/issues/new/choose) and include:

- the edition (desktop app, PowerShell or Linux) and the version
- your Windows build or Linux distribution
- what you did, what you expected and what happened
- the log lines from the operation center or the log file, if there are any

Do not post exploit details. Read [SECURITY.md](SECURITY.md) for security problems.

## Propose a tweak

A new tweak needs:

- the exact registry values, services or scheduled tasks it changes
- the original value it should save, and how undo works
- a risk level (Safe, Moderate or Advanced) and a reason for it
- a source or a test that shows the setting does what the description says

Tweaks that change security protections, or that claim a speed gain without a measurement, are not accepted.

## Pull requests

- Keep a change to one topic.
- Desktop app (`desktop/`): run `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings` and `cargo test --locked` before you push. Tests that would change a real system use the fake system in `src/opti/backend.rs`.
- PowerShell edition: run `tests\Optimaxer.Tests.ps1`.
- Linux edition: run `python -B -m unittest discover -s linux/tests`.
- Describe what you ran and what you did not run.

## Licenses

The PowerShell and Linux editions use MIT. The `desktop/` folder derives from Orion Store and uses GPL-3.0. Your contribution uses the license of the folder it changes.
