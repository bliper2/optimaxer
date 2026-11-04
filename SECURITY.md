# Security policy

Optimaxer changes sensitive Windows settings, so security reports get priority.

## Report a vulnerability

Do not post exploit details in a public issue.

1. Use **Report a vulnerability** on the repository's [Security tab](https://github.com/bliper2/optimaxer/security) if the button appears.
2. If it does not, open an [issue](https://github.com/bliper2/optimaxer/issues/new) titled "Security contact request" with no technical detail. The maintainer will arrange a private channel.

Include the edition and version, the steps to reproduce, and what an attacker could do. You will get a reply on the issue. Fixes ship as a new release and the release notes say what changed.

## Supported versions

Only the latest release receives fixes.

## How downloads are checked

- Releases publish `SHA256SUMS.txt` and GitHub shows a SHA-256 digest for each asset. Compare it with your download.
- The desktop updater downloads only after you press the update button, checks the file against the digest GitHub publishes, and refuses a release without one.
- The PowerShell installer and updater check the published SHA-256. The updater rejects archives with paths that escape the install folder.
- The desktop exe is not code-signed, so Windows SmartScreen may warn. You can build it from source with `cargo build --release` in `desktop/`.

More detail is on the [security page](https://bliper2.github.io/optimaxer/security.html). The [privacy policy](https://bliper2.github.io/optimaxer/privacy.html) lists every network request the software makes.
