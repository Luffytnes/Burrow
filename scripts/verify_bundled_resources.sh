#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RESOURCE_DIR="$SCRIPT_DIR/../src-tauri/resources"
CLAMAV_DIR="$RESOURCE_DIR/clamav"
MOLE_DIR="$RESOURCE_DIR/mole"
SOURCE_DIR="$SCRIPT_DIR/../third_party/sources"

for command_name in bash codesign file grep gzip otool shasum; do
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
test -s "$SOURCE_DIR/clamav-1.5.4.tar.gz"
test -s "$SOURCE_DIR/SHA256SUMS"

if ! grep -q "GNU GENERAL PUBLIC LICENSE" "$MOLE_DIR/LICENSE"; then
    echo "Bundled Mole must include its GNU GPL license." >&2
    exit 1
fi

grep -qx "Mole 1.48.1" "$MOLE_DIR/VERSION"
test -s "$SOURCE_DIR/mole-1.48.1.tar.gz"

bash -n "$MOLE_DIR/bin/mo" "$MOLE_DIR/bin/mole" "$MOLE_DIR/libexec/mole"
while IFS= read -r script_path; do
    bash -n "$script_path"
done < <(find "$MOLE_DIR/libexec/bin" "$MOLE_DIR/libexec/lib" -type f -name '*.sh' -print)

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
(
    cd "$SOURCE_DIR"
    shasum -a 256 --check SHA256SUMS
    gzip -t clamav-1.5.4.tar.gz
    gzip -t mole-1.48.1.tar.gz
)

for binary in \
    "$CLAMAV_DIR/bin/clamscan" \
    "$CLAMAV_DIR/bin/freshclam" \
    "$CLAMAV_DIR"/lib/*.dylib \
    "$MOLE_DIR/libexec/bin/analyze-go" \
    "$MOLE_DIR/libexec/bin/status-go" \
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
