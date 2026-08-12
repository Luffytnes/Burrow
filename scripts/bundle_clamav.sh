#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FINAL_DEST="$SCRIPT_DIR/../src-tauri/resources/clamav"
BUILD_DIR="$(mktemp -d "$SCRIPT_DIR/../src-tauri/resources/.clamav-build.XXXXXX")"
trap 'rm -rf "$BUILD_DIR"' EXIT
DEST="$BUILD_DIR/clamav"
BIN_DIR="$DEST/bin"
LIB_DIR="$DEST/lib"
LICENSE_DIR="$DEST/licenses"

require_command() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "✗ Required command not found: $1" >&2
        exit 1
    }
}

for command_name in brew install_name_tool otool codesign shasum file lipo; do
    require_command "$command_name"
done

CLAMAV_PREFIX="$(brew --prefix clamav)"
PCRE2_PREFIX="$(brew --prefix pcre2)"
JSONC_PREFIX="$(brew --prefix json-c)"
OPENSSL_PREFIX="$(brew --prefix openssl@3)"

CLAMAV_LIB="$CLAMAV_PREFIX/lib"
CLAMAV_BIN="$CLAMAV_PREFIX/bin"
PCRE2_LIB="$PCRE2_PREFIX/lib"
JSONC_LIB="$JSONC_PREFIX/lib"
OPENSSL_LIB="$OPENSSL_PREFIX/lib"

echo "→ Destination: $FINAL_DEST"
mkdir -p "$BIN_DIR" "$LIB_DIR" "$LICENSE_DIR"

copy_required() {
    local source="$1"
    local destination="$2"
    if [[ ! -f "$source" ]]; then
        echo "✗ Missing required file: $source" >&2
        exit 1
    fi
    cp -f "$source" "$destination"
}

echo "→ Copying ClamAV binaries and runtime libraries…"
copy_required "$CLAMAV_BIN/clamscan" "$BIN_DIR/clamscan"
copy_required "$CLAMAV_BIN/freshclam" "$BIN_DIR/freshclam"
copy_required "$CLAMAV_LIB/libclamav.12.dylib" "$LIB_DIR/libclamav.12.dylib"
copy_required "$CLAMAV_LIB/libclammspack.0.dylib" "$LIB_DIR/libclammspack.0.dylib"
copy_required "$CLAMAV_LIB/libfreshclam.4.dylib" "$LIB_DIR/libfreshclam.4.dylib"
copy_required "$PCRE2_LIB/libpcre2-8.0.dylib" "$LIB_DIR/libpcre2-8.0.dylib"
copy_required "$JSONC_LIB/libjson-c.5.dylib" "$LIB_DIR/libjson-c.5.dylib"
copy_required "$OPENSSL_LIB/libssl.3.dylib" "$LIB_DIR/libssl.3.dylib"
copy_required "$OPENSSL_LIB/libcrypto.3.dylib" "$LIB_DIR/libcrypto.3.dylib"
chmod +x "$BIN_DIR/clamscan" "$BIN_DIR/freshclam"
chmod -R u+w "$BIN_DIR" "$LIB_DIR"

echo "→ Copying license texts…"
copy_required "$CLAMAV_PREFIX/COPYING.txt" "$LICENSE_DIR/ClamAV-GPL-2.0-or-later.txt"
copy_required "$PCRE2_PREFIX/LICENCE.md" "$LICENSE_DIR/PCRE2-BSD-3-Clause.md"
copy_required "$JSONC_PREFIX/COPYING" "$LICENSE_DIR/json-c-MIT.txt"
copy_required "$OPENSSL_PREFIX/LICENSE.txt" "$LICENSE_DIR/OpenSSL-Apache-2.0.txt"

remove_cellar_rpaths() {
    local binary="$1"
    while IFS= read -r rpath; do
        [[ -n "$rpath" ]] || continue
        install_name_tool -delete_rpath "$rpath" "$binary" 2>/dev/null || true
    done < <(
        otool -l "$binary" |
            awk '/cmd LC_RPATH/{seen=1; next} seen && /path /{print $2; seen=0}' |
            grep '/Cellar/' || true
    )
}

rewrite_dependencies() {
    local binary="$1"
    local prefix="$2"
    while IFS= read -r dependency; do
        [[ -n "$dependency" ]] || continue
        local name
        name="$(basename "$dependency")"
        case "$name" in
            libclamav.12.dylib|libclammspack.0.dylib|libfreshclam.4.dylib|\
            libpcre2-8.0.dylib|libjson-c.5.dylib|libssl.3.dylib|libcrypto.3.dylib)
                install_name_tool -change "$dependency" "$prefix/$name" "$binary" 2>/dev/null || true
                ;;
        esac
    done < <(otool -L "$binary" | tail -n +2 | awk '{print $1}')
}

fix_executable() {
    local binary="$1"
    echo "  fixing executable: $(basename "$binary")"
    remove_cellar_rpaths "$binary"
    install_name_tool -add_rpath "@executable_path/../lib" "$binary" 2>/dev/null || true
    rewrite_dependencies "$binary" "@executable_path/../lib"
}

fix_library() {
    local library="$1"
    local name
    name="$(basename "$library")"
    echo "  fixing library: $name"
    install_name_tool -id "@rpath/$name" "$library"
    remove_cellar_rpaths "$library"
    install_name_tool -add_rpath "@loader_path" "$library" 2>/dev/null || true
    rewrite_dependencies "$library" "@loader_path"
}

echo "→ Rewriting Mach-O paths…"
fix_executable "$BIN_DIR/clamscan"
fix_executable "$BIN_DIR/freshclam"
for library in "$LIB_DIR"/*.dylib; do
    fix_library "$library"
done

echo "→ Verifying Apple Silicon architecture…"
for binary in "$BIN_DIR"/* "$LIB_DIR"/*.dylib; do
    architectures="$(lipo -archs "$binary")"
    if [[ "$architectures" != "arm64" ]]; then
        echo "✗ Expected arm64-only binary, got '$architectures': $binary" >&2
        exit 1
    fi
done

# install_name_tool invalidates upstream signatures. Ad-hoc signatures keep the
# development bundle runnable; release signing is applied to the final app.
echo "→ Applying development ad-hoc signatures…"
for library in "$LIB_DIR"/*.dylib; do
    codesign --sign - --force "$library"
done
codesign --sign - --force "$BIN_DIR/clamscan"
codesign --sign - --force "$BIN_DIR/freshclam"

version="$($BIN_DIR/clamscan --version | awk '{print $2; exit}')"
printf 'ClamAV %s\n' "$version" > "$DEST/VERSION"

(
    cd "$DEST"
    shasum -a 256 bin/* lib/*.dylib | LC_ALL=C sort > SHA256SUMS
)

echo "→ Checking for unresolved package-manager paths…"
remaining="$(
    otool -L "$BIN_DIR"/* "$LIB_DIR"/*.dylib |
        grep -E '/opt/homebrew|/usr/local/Cellar' || true
)"
if [[ -n "$remaining" ]]; then
    echo "✗ Remaining non-relocatable references:" >&2
    echo "$remaining" >&2
    exit 1
fi

backup="$BUILD_DIR/original-clamav"
if [[ -d "$FINAL_DEST" ]]; then
    mv "$FINAL_DEST" "$backup"
fi
if ! mv "$DEST" "$FINAL_DEST"; then
    if [[ -d "$backup" ]]; then
        mv "$backup" "$FINAL_DEST"
    fi
    echo "✗ Failed to replace the bundled ClamAV directory" >&2
    exit 1
fi
rm -rf "$backup"

echo "✓ Bundled ClamAV $version for arm64"
