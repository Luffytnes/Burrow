#!/bin/sh
set -eu

PROJECT_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
CACHE_DIR="${TMPDIR:-/tmp}/burrow-clang-module-cache"
GENERATOR="${TMPDIR:-/tmp}/burrow-macos-icon-generator"

mkdir -p "$CACHE_DIR"

clang \
  -fobjc-arc \
  -fmodules \
  -fmodules-cache-path="$CACHE_DIR" \
  -Wall \
  -Wextra \
  -Werror \
  -framework AppKit \
  -framework ImageIO \
  "$PROJECT_ROOT/scripts/generate_macos_icon.m" \
  -o "$GENERATOR"

cd "$PROJECT_ROOT"
"$GENERATOR" \
  src-tauri/icons/burrow.png \
  src-tauri/icons/icon.icns \
  src-tauri/icons/macos-icon.png
