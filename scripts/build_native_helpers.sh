#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RESOURCE_DIR="$SCRIPT_DIR/../src-tauri/resources"
BUILD_DIR="$(mktemp -d)"
trap 'rm -rf "$BUILD_DIR"' EXIT

for command_name in xcrun codesign lipo shasum; do
    command -v "$command_name" >/dev/null 2>&1 || {
        echo "Missing required command: $command_name" >&2
        exit 1
    }
done

env CLANG_MODULE_CACHE_PATH="$BUILD_DIR/module-cache" xcrun --sdk macosx swiftc \
    -target arm64-apple-macosx12.0 \
    "$RESOURCE_DIR/burrow-smc.swift" \
    -o "$BUILD_DIR/burrow-smc"

env CLANG_MODULE_CACHE_PATH="$BUILD_DIR/module-cache" xcrun --sdk macosx clang \
    -arch arm64 \
    -mmacosx-version-min=12.0 \
    -fobjc-arc \
    -framework Foundation \
    -framework LocalAuthentication \
    "$RESOURCE_DIR/burrow-touchid.m" \
    -o "$BUILD_DIR/burrow-touchid"

for helper in burrow-smc burrow-touchid; do
    if [[ "$(lipo -archs "$BUILD_DIR/$helper")" != "arm64" ]]; then
        echo "Expected arm64-only helper: $helper" >&2
        exit 1
    fi
    codesign --sign - --force "$BUILD_DIR/$helper"
done

for helper in burrow-smc burrow-touchid; do
    mv -f "$BUILD_DIR/$helper" "$RESOURCE_DIR/$helper"
done

(
    cd "$RESOURCE_DIR"
    shasum -a 256 burrow-smc burrow-touchid | LC_ALL=C sort > SHA256SUMS
)

echo "Built and signed Burrow native helpers for arm64."
