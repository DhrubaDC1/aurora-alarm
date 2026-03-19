#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
APPDIR="$ROOT/packaging/appimage/AppDir"
RELEASE_DIR="$ROOT/release"
CACHE_DIR="$ROOT/.cache/appimage"
RAW_VERSION="${GITHUB_REF_NAME:-${VERSION:-0.1.1}}"
VERSION="${RAW_VERSION#v}"

mkdir -p "$APPDIR/usr/bin" \
  "$APPDIR/usr/share/applications" \
  "$APPDIR/usr/share/icons/hicolor/scalable/apps" \
  "$APPDIR/usr/share/metainfo" \
  "$RELEASE_DIR" \
  "$CACHE_DIR"

install -m755 "$ROOT/target/release/alarm-app" "$APPDIR/usr/bin/alarm-app"
install -m755 "$ROOT/target/release/alarm-daemon" "$APPDIR/usr/bin/alarm-daemon"
install -m755 "$ROOT/packaging/appimage/AppRun" "$APPDIR/AppRun"
install -m644 "$ROOT/dist/linux/io.codex.AuroraAlarm.desktop" "$APPDIR/usr/share/applications/io.codex.AuroraAlarm.desktop"
install -m644 "$ROOT/dist/linux/io.codex.AuroraAlarm.metainfo.xml" "$APPDIR/usr/share/metainfo/io.codex.AuroraAlarm.metainfo.xml"
install -m644 "$ROOT/dist/icons/io.codex.AuroraAlarm.svg" "$APPDIR/usr/share/icons/hicolor/scalable/apps/io.codex.AuroraAlarm.svg"

cp "$ROOT/dist/icons/io.codex.AuroraAlarm.svg" "$APPDIR/io.codex.AuroraAlarm.svg"
cp "$ROOT/dist/linux/io.codex.AuroraAlarm.desktop" "$APPDIR/io.codex.AuroraAlarm.desktop"

LINUXDEPLOY="$CACHE_DIR/linuxdeploy-x86_64.AppImage"
APPIMAGE_PLUGIN="$CACHE_DIR/linuxdeploy-plugin-appimage-x86_64.AppImage"

if [[ ! -f "$LINUXDEPLOY" ]]; then
  curl -L "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage" -o "$LINUXDEPLOY"
  chmod +x "$LINUXDEPLOY"
fi

if [[ ! -f "$APPIMAGE_PLUGIN" ]]; then
  curl -L "https://github.com/linuxdeploy/linuxdeploy-plugin-appimage/releases/download/continuous/linuxdeploy-plugin-appimage-x86_64.AppImage" -o "$APPIMAGE_PLUGIN"
  chmod +x "$APPIMAGE_PLUGIN"
fi

export VERSION
export ARCH=x86_64
export OUTPUT="$RELEASE_DIR/AuroraAlarm-${VERSION}-x86_64.AppImage"

"$LINUXDEPLOY" \
  --appdir "$APPDIR" \
  --desktop-file "$ROOT/dist/linux/io.codex.AuroraAlarm.desktop" \
  --icon-file "$ROOT/dist/icons/io.codex.AuroraAlarm.svg" \
  --executable "$ROOT/target/release/alarm-app" \
  --output appimage

mv ./*.AppImage "$OUTPUT"
