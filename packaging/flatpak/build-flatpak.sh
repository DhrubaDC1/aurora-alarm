#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BUILD_DIR="$ROOT/.flatpak-builder"
REPO_DIR="$ROOT/.flatpak-repo"
RELEASE_DIR="$ROOT/release"
VENDOR_DIR="$ROOT/packaging/flatpak/vendor"
VERSION="${GITHUB_REF_NAME:-${VERSION:-0.1.0}}"

mkdir -p "$RELEASE_DIR"
rm -rf "$VENDOR_DIR"
cargo vendor --versioned-dirs "$VENDOR_DIR"

flatpak-builder \
  --force-clean \
  --repo="$REPO_DIR" \
  "$BUILD_DIR" \
  "$ROOT/packaging/flatpak/io.codex.AuroraAlarm.yml"

flatpak build-bundle \
  "$REPO_DIR" \
  "$RELEASE_DIR/AuroraAlarm-${VERSION}-x86_64.flatpak" \
  io.codex.AuroraAlarm \
  stable
