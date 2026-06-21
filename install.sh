#!/bin/sh
# FileFlow installer — fetches the latest release and installs it to /Applications.
#
#   curl -fsSL https://raw.githubusercontent.com/Arylmera/FileFlow/main/install.sh | sh
#
# Why this skips Gatekeeper: macOS only quarantines files a *browser* downloads.
# A build delivered over curl carries no com.apple.quarantine flag, so the
# (ad-hoc signed) app launches with no prompt. The xattr below is just insurance.
set -eu

REPO="Arylmera/FileFlow"
APP="FileFlow.app"
ASSET="FileFlow.app.tar.gz"
DEST="/Applications"

[ "$(uname)" = "Darwin" ] || { echo "FileFlow is macOS-only." >&2; exit 1; }

echo "==> Finding the latest FileFlow release…"
URL=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
  | grep -o "https://[^\"]*$ASSET" | head -1)
[ -n "${URL:-}" ] || {
  echo "No '$ASSET' in the latest release of $REPO. Has a release been published?" >&2
  exit 1
}

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

echo "==> Downloading…"
curl -fsSL "$URL" -o "$TMP/$ASSET"

echo "==> Unpacking…"
tar -xzf "$TMP/$ASSET" -C "$TMP"
[ -d "$TMP/$APP" ] || { echo "Archive did not contain $APP." >&2; exit 1; }

# /Applications is writable by admins without sudo on a stock macOS; fall back if not.
SUDO=""
[ -w "$DEST" ] || SUDO="sudo"

echo "==> Installing to $DEST/$APP…"
$SUDO rm -rf "$DEST/$APP"
$SUDO mv "$TMP/$APP" "$DEST/$APP"
$SUDO xattr -dr com.apple.quarantine "$DEST/$APP" 2>/dev/null || true

echo "==> Launching FileFlow…"
open "$DEST/$APP"
echo "Done — FileFlow now lives in your menu bar."
