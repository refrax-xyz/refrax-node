#!/bin/sh
# Install the `refrax` binary from the latest GitHub release.
#
#   curl -fsSL https://raw.githubusercontent.com/refrax-xyz/refrax-node/main/install.sh | sh
#
# Options (environment variables):
#   REFRAX_VERSION      a tag such as v0.1.0 (default: latest release)
#   REFRAX_INSTALL_DIR  where to put the binary (default: ~/.local/bin)
#
# The archive's SHA-256 is checked against the .sha256 file published next to it.
set -eu

REPO="${REFRAX_REPO:-refrax-xyz/refrax-node}"
BASE="${REFRAX_BASE_URL:-https://github.com/$REPO/releases}"
VERSION="${REFRAX_VERSION:-latest}"
DIR="${REFRAX_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf '  \033[38;5;141m◆\033[0m %s\n' "$1"; }
die() { printf '  \033[31m✕\033[0m %s\n' "$1" >&2; exit 1; }

case "$(uname -s)" in
  Linux)  os=unknown-linux-gnu ;;
  Darwin) os=apple-darwin ;;
  *) die "unsupported OS $(uname -s). On Windows download the .zip from $BASE, or build with cargo." ;;
esac
case "$(uname -m)" in
  x86_64 | amd64)  arch=x86_64 ;;
  aarch64 | arm64) arch=aarch64 ;;
  *) die "unsupported CPU $(uname -m). Build from source: cargo install --git https://github.com/$REPO refrax-cli" ;;
esac
target="$arch-$os"
asset="refrax-$target.tar.gz"
if [ "$VERSION" = latest ]; then url="$BASE/latest/download/$asset"; else url="$BASE/download/$VERSION/$asset"; fi

if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -qO "$2" "$1"; }
else
  die "need curl or wget"
fi
if command -v sha256sum >/dev/null 2>&1; then
  sha() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
  sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  die "need sha256sum or shasum to verify the download"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

say "downloading $asset ($VERSION)"
fetch "$url" "$tmp/$asset" || die "download failed: $url"
fetch "$url.sha256" "$tmp/$asset.sha256" || die "checksum download failed: $url.sha256"

want="$(cut -d' ' -f1 < "$tmp/$asset.sha256")"
got="$(sha "$tmp/$asset")"
[ "$want" = "$got" ] || die "checksum mismatch for $asset (expected $want, got $got)"
say "sha256 ok  $got"

tar -xzf "$tmp/$asset" -C "$tmp"
mkdir -p "$DIR"
install -m 0755 "$tmp/refrax-$target/refrax" "$DIR/refrax" 2>/dev/null \
  || { cp "$tmp/refrax-$target/refrax" "$DIR/refrax" && chmod 0755 "$DIR/refrax"; }
say "installed  $DIR/refrax"

case ":$PATH:" in
  *":$DIR:"*) ;;
  *) say "add it to your PATH:  export PATH=\"$DIR:\$PATH\"" ;;
esac
say "next       refrax devnet up"
