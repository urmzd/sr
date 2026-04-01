#!/bin/sh
# download-sr.sh — Downloads the sr binary from GitHub releases.
#
# Environment variables (required):
#   SR_REPO    — repository slug (e.g. "urmzd/sr")
#   SR_REF     — release tag to download (e.g. "v1.2.0")
#   SR_DEST    — destination path for the binary
#
# Environment variables (optional):
#   SR_SHA256    — expected SHA256 checksum; skips verification if unset
#   GH_TOKEN     — GitHub token; used by both gh CLI and curl fallback
#   GITHUB_SERVER_URL — base URL (defaults to https://github.com); set
#                       automatically by GitHub Actions on GHES

set -eu

: "${SR_REPO:?SR_REPO must be set}"
: "${SR_REF:?SR_REF must be set}"
: "${SR_DEST:?SR_DEST must be set}"

OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$ARCH" in
  x86_64)        ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *) echo "Unsupported architecture: $ARCH" >&2; exit 1 ;;
esac

case "$OS" in
  linux)  TARGET="${ARCH}-unknown-linux-musl" ;;
  darwin) TARGET="${ARCH}-apple-darwin" ;;
  *) echo "Unsupported OS: $OS" >&2; exit 1 ;;
esac

BINARY_NAME="sr-${TARGET}"
mkdir -p "$(dirname "$SR_DEST")"

if command -v gh >/dev/null 2>&1; then
  echo "Downloading $BINARY_NAME via gh CLI..."
  gh release download "$SR_REF" \
    --repo "$SR_REPO" \
    --pattern "$BINARY_NAME" \
    --output "$SR_DEST"
else
  SERVER_URL="${GITHUB_SERVER_URL:-https://github.com}"
  DOWNLOAD_URL="${SERVER_URL}/${SR_REPO}/releases/download/${SR_REF}/${BINARY_NAME}"
  echo "gh CLI not found; downloading $BINARY_NAME via curl..."
  echo "URL: $DOWNLOAD_URL"

  if [ -n "${GH_TOKEN:-}" ]; then
    curl -fsSL -H "Authorization: token $GH_TOKEN" "$DOWNLOAD_URL" -o "$SR_DEST"
  else
    curl -fsSL "$DOWNLOAD_URL" -o "$SR_DEST"
  fi
fi

if [ -n "${SR_SHA256:-}" ]; then
  if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL=$(sha256sum "$SR_DEST" | awk '{print $1}')
  elif command -v shasum >/dev/null 2>&1; then
    ACTUAL=$(shasum -a 256 "$SR_DEST" | awk '{print $1}')
  else
    echo "sha256sum or shasum required for checksum verification" >&2
    exit 1
  fi
  if [ "$ACTUAL" != "$SR_SHA256" ]; then
    echo "SHA256 mismatch: expected $SR_SHA256, got $ACTUAL" >&2
    exit 1
  fi
  echo "SHA256 verified: $ACTUAL"
fi

chmod +x "$SR_DEST"
