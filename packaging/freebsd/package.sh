#!/usr/bin/env bash
# Build and package DeckCraft for FreeBSD:
#
#   $DIST/deckcraft-<version>-freebsd-<arch>.tar.gz   bin/, share/ (desktop entry, icons, AppStream)
#
# Usage: packaging/freebsd/package.sh [--skip-build]
#
# Run on FreeBSD (CI: a FreeBSD 14 VM via vmactions/freebsd-vm). Runtime needs: libxkbcommon,
# wayland or libX11, mesa (Vulkan or EGL) and alsa-lib:
# `pkg install libxkbcommon libX11 mesa-libs vulkan-loader alsa-lib`.
set -euo pipefail
# shellcheck source=../common.sh
. "$(dirname "${BASH_SOURCE[0]}")/../common.sh"
LINUX="$ROOT/packaging/linux"
APP_ID=ai.storyteller.deckcraft

TARGET_ARGS=()
BIN="$CARGO_TARGET_DIR/release"
# Cross builds (CARGO_BUILD_TARGET from the release workflow) land under target/<triple>/release.
if [[ -n "${CARGO_BUILD_TARGET:-}" ]]; then
  TARGET_ARGS=(--target "$CARGO_BUILD_TARGET")
  BIN="$CARGO_TARGET_DIR/$CARGO_BUILD_TARGET/release"
fi

if [[ "${1:-}" != "--skip-build" ]]; then
  (cd "$ROOT" && cargo build --release --locked "${TARGET_ARGS[@]}" -p deckcraft -p deckcraft-cli)
fi

if [[ -n "${CARGO_BUILD_TARGET:-}" ]]; then
  # The target triple, not the build host's uname, names the arch of a cross build.
  if [[ "$CARGO_BUILD_TARGET" == aarch64-unknown-freebsd ]]; then ARCH=aarch64; else ARCH=x86_64; fi
else
  ARCH="$(uname -m)"
  case "$ARCH" in
    amd64) ARCH=x86_64 ;;
    arm64) ARCH=aarch64 ;;
    *) ;;
  esac
fi
NAME="deckcraft-$VERSION-freebsd-$ARCH"
WORK="$CARGO_TARGET_DIR/freebsd-package"
STAGE="$WORK/$NAME"
rm -rf "$WORK"
mkdir -p "$STAGE/bin" "$STAGE/share/applications" "$STAGE/share/metainfo" "$STAGE/share/mime/packages" "$STAGE/share/icons" "$STAGE/share/doc/deckcraft"

install -m 755 "$BIN/deckcraft" "$BIN/deckcraft-cli" "$STAGE/bin/"
strip "$STAGE/bin/deckcraft" "$STAGE/bin/deckcraft-cli" 2>/dev/null || true
install -m 644 "$LINUX/$APP_ID.desktop" "$STAGE/share/applications/"
install -m 644 "$LINUX/$APP_ID.mime.xml" "$STAGE/share/mime/packages/$APP_ID.xml"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$DECKCRAFT_BUILD_DATE/g" \
  "$LINUX/$APP_ID.metainfo.xml.in" >"$STAGE/share/metainfo/$APP_ID.metainfo.xml"
cp -R "$ROOT/assets/app-icon/hicolor" "$STAGE/share/icons/"
copy_docs "$STAGE/share/doc/deckcraft"

tar -C "$WORK" -czf "$DIST/$NAME.tar.gz" "$NAME"
if [[ -z "${CARGO_BUILD_TARGET:-}" ]]; then "$STAGE/bin/deckcraft-cli" --version; fi # a cross-built binary cannot run on the build host
echo "wrote $DIST/$NAME.tar.gz"
ls -lh "$DIST/$NAME.tar.gz"
