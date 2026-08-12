#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RESOURCE_DIR="$SCRIPT_DIR/../src-tauri/resources"
CLAMAV_DIR="$RESOURCE_DIR/clamav"
MOLE_DIR="$RESOURCE_DIR/mole"

for command_name in codesign file grep otool shasum; do
    command -v "$command_name" >/dev/null 2>&1 || {
        echo "Missing required command: $command_name" >&2
        exit 1
    }
done

test -s "$CLAMAV_DIR/VERSION"
test -s "$CLAMAV_DIR/SHA256SUMS"
test -s "$MOLE_DIR/VERSION"
test -s "$MOLE_DIR/SHA256SUMS"
test -s "$MOLE_DIR/LICENSE"
test -s "$SCRIPT_DIR/../LICENSE"
test -s "$SCRIPT_DIR/../THIRD_PARTY_NOTICES.md"

(
    cd "$CLAMAV_DIR"
    shasum -a 256 --check SHA256SUMS
)
(
    cd "$MOLE_DIR"
    shasum -a 256 --check SHA256SUMS
)
(
    cd "$RESOURCE_DIR"
    shasum -a 256 --check SHA256SUMS
)

for binary in \
    "$CLAMAV_DIR/bin/clamscan" \
    "$CLAMAV_DIR/bin/freshclam" \
    "$CLAMAV_DIR"/lib/*.dylib \
    "$RESOURCE_DIR/burrow-smc" \
    "$RESOURCE_DIR/burrow-touchid"; do
    file "$binary" | grep -q 'arm64'
    codesign --verify --strict "$binary"
done

if otool -L "$CLAMAV_DIR"/bin/* "$CLAMAV_DIR"/lib/*.dylib | \
    grep -Eq '/opt/homebrew|/usr/local/Cellar'; then
    echo "Non-relocatable Homebrew path found in bundled ClamAV." >&2
    exit 1
fi

clamav_version="$($CLAMAV_DIR/bin/clamscan --version)"
mole_version="$($MOLE_DIR/bin/mo --version)"
grep -Fq "$(cut -d' ' -f2 "$CLAMAV_DIR/VERSION")" <<< "$clamav_version"
grep -Fq "$(cut -d' ' -f2 "$MOLE_DIR/VERSION")" <<< "$mole_version"

echo "Bundled resource integrity checks passed."
