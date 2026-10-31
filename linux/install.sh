#!/usr/bin/env bash
# Optimaxer for Manjaro / Arch Linux and Fedora
#   curl -fsSL https://raw.githubusercontent.com/bliper2/optimaxer/main/linux/install.sh | bash
# Downloads the latest release binary, verifies its SHA-256, installs it to ~/.local/bin/optimaxer.
set -euo pipefail
REPO="bliper2/optimaxer"
DEST="${OPTIMAXER_DEST:-$HOME/.local/bin}"
ASSET="optimaxer-linux-$(uname -m)"

need() { command -v "$1" >/dev/null 2>&1 || { echo "optimaxer: '$1' is required" >&2; exit 1; }; }
need curl
need sha256sum
if ! grep -qiE 'manjaro|arch|fedora|rhel|centos|rocky|alma|nobara' /etc/os-release 2>/dev/null; then
    echo "warning: this edition supports Manjaro and other Arch-based distributions (pacman) and Fedora and other dnf-based distributions." >&2
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Looking up the latest release..."
curl -fsSL -H 'Accept: application/vnd.github+json' "https://api.github.com/repos/$REPO/releases/latest" -o "$tmp/rel.json"
tag="$(grep -m1 '"tag_name"' "$tmp/rel.json" | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')"
# the asset object lists name, then digest and browser_download_url; read the lines of the matching asset only
block="$(grep -n "\"name\": \"$ASSET\"" "$tmp/rel.json" | head -1 | cut -d: -f1 || true)"
[[ -n "$block" ]] || { echo "the latest release has no $ASSET asset" >&2; exit 1; }
assetjson="$(sed -n "$((block - 12)),$((block + 25))p" "$tmp/rel.json")"
url="$(printf '%s\n' "$assetjson" | grep -m1 '"browser_download_url"' | sed -E 's/.*"browser_download_url": *"([^"]+)".*/\1/')"
digest="$(printf '%s\n' "$assetjson" | grep -m1 '"digest"' | sed -E 's/.*"digest": *"([^"]+)".*/\1/' || true)"
echo "Latest is $tag"

curl -fsSL "$url" -o "$tmp/optimaxer"
if [[ "$digest" == sha256:* ]]; then
    got="$(sha256sum "$tmp/optimaxer" | cut -d' ' -f1)"
    [[ "sha256:$got" == "$digest" ]] || { echo "checksum mismatch, aborting" >&2; exit 1; }
    echo "Checksum verified (SHA-256)."
else
    echo "No valid published SHA-256 checksum for this release; aborting." >&2
    exit 1
fi
[[ "$(head -c4 "$tmp/optimaxer" | od -An -c | tr -d ' ')" == '177ELF' ]] || { echo "not a Linux executable, aborting" >&2; exit 1; }

mkdir -p "$DEST"
install -m 0755 "$tmp/optimaxer" "$DEST/optimaxer"
echo "Installed to $DEST/optimaxer"
case ":$PATH:" in
    *":$DEST:"*) ;;
    *) echo "Note: add $DEST to your PATH (for example: echo 'export PATH=\"$DEST:\$PATH\"' >> ~/.bashrc)" ;;
esac
echo
"$DEST/optimaxer" launcher || true
echo "Run  optimaxer  (window)  or  optimaxer --text  (terminal menu)."
