#!/usr/bin/env bash
# Optimaxer for Manjaro / Arch Linux
#   curl -fsSL https://raw.githubusercontent.com/bliper2/optimaxer/main/linux/install.sh | bash
# Downloads the latest release script, verifies its SHA-256, installs it to ~/.local/bin/optimaxer.
set -euo pipefail
REPO="bliper2/optimaxer"
DEST="${OPTIMAXER_DEST:-$HOME/.local/bin}"

need() { command -v "$1" >/dev/null 2>&1 || { echo "optimaxer: '$1' is required" >&2; exit 1; }; }
need curl
need python3
if ! grep -qiE 'manjaro|arch' /etc/os-release 2>/dev/null; then
    echo "warning: this edition targets Manjaro and other Arch-based distributions (pacman)." >&2
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Looking up the latest release..."
curl -fsSL -H 'Accept: application/vnd.github+json' "https://api.github.com/repos/$REPO/releases/latest" -o "$tmp/rel.json"
read -r url digest tag < <(python3 - "$tmp/rel.json" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))
a = next((a for a in r.get("assets", []) if a["name"] == "optimaxer-linux.py"), None)
if not a:
    sys.exit("the latest release has no optimaxer-linux.py asset")
print(a["browser_download_url"], a.get("digest") or "-", r["tag_name"])
PY
)
echo "Latest is $tag"

curl -fsSL "$url" -o "$tmp/optimaxer.py"
if [[ "$digest" == sha256:* ]]; then
    got="$(sha256sum "$tmp/optimaxer.py" | cut -d' ' -f1)"
    [[ "sha256:$got" == "$digest" ]] || { echo "checksum mismatch, aborting" >&2; exit 1; }
    echo "Checksum verified (SHA-256)."
else
    echo "No published checksum for this release; relying on HTTPS." >&2
fi
python3 -c 'import ast, sys; ast.parse(open(sys.argv[1]).read())' "$tmp/optimaxer.py"

mkdir -p "$DEST"
install -m 0755 "$tmp/optimaxer.py" "$DEST/optimaxer"
echo "Installed to $DEST/optimaxer"
case ":$PATH:" in
    *":$DEST:"*) ;;
    *) echo "Note: add $DEST to your PATH (for example: echo 'export PATH=\"$DEST:\$PATH\"' >> ~/.bashrc)" ;;
esac
echo
echo "Try:  optimaxer --dry-run        (interactive menu, nothing is changed)"
echo "      optimaxer                  (interactive menu; asks for sudo when needed)"
echo "      optimaxer gui              (graphical window; also in your application menu as Optimaxer)"
"$DEST/optimaxer" launcher >/dev/null 2>&1 || true
python3 -c 'import tkinter' >/dev/null 2>&1 || echo "Note: the graphical window needs Tk:  sudo pacman -S tk"
