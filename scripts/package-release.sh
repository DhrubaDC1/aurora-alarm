#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${GITHUB_REF_NAME:-${VERSION:-0.1.0}}"
STAGE_DIR="$ROOT/release/stage"
ARCHIVE_ROOT="$STAGE_DIR/aurora-alarm-${VERSION}-linux-x86_64"
RELEASE_DIR="$ROOT/release"

rm -rf "$STAGE_DIR"
mkdir -p "$ARCHIVE_ROOT/bin" "$ARCHIVE_ROOT/share/applications" "$ARCHIVE_ROOT/share/icons/hicolor/scalable/apps" "$ARCHIVE_ROOT/share/metainfo" "$RELEASE_DIR"

install -m755 "$ROOT/target/release/alarm-app" "$ARCHIVE_ROOT/bin/alarm-app"
install -m755 "$ROOT/target/release/alarm-daemon" "$ARCHIVE_ROOT/bin/alarm-daemon"
install -m644 "$ROOT/dist/linux/io.codex.AuroraAlarm.desktop" "$ARCHIVE_ROOT/share/applications/io.codex.AuroraAlarm.desktop"
install -m644 "$ROOT/dist/linux/io.codex.AuroraAlarm.metainfo.xml" "$ARCHIVE_ROOT/share/metainfo/io.codex.AuroraAlarm.metainfo.xml"
install -m644 "$ROOT/dist/icons/io.codex.AuroraAlarm.svg" "$ARCHIVE_ROOT/share/icons/hicolor/scalable/apps/io.codex.AuroraAlarm.svg"
install -m644 "$ROOT/dist/systemd/aurora-alarm-daemon.service" "$ARCHIVE_ROOT/aurora-alarm-daemon.service"
install -m644 "$ROOT/README.md" "$ARCHIVE_ROOT/README.md"
install -m644 "$ROOT/LICENSE" "$ARCHIVE_ROOT/LICENSE"

tar -C "$STAGE_DIR" -czf "$RELEASE_DIR/aurora-alarm-${VERSION}-linux-x86_64.tar.gz" "aurora-alarm-${VERSION}-linux-x86_64"
