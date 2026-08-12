#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST="$SCRIPT_DIR/../src-tauri/resources/clamav"

BIN_DIR="$DEST/bin"
LIB_DIR="$DEST/lib"

CLAMAV_LIB="/opt/homebrew/opt/clamav/lib"
CLAMAV_BIN="/opt/homebrew/bin"
PCRE2_LIB="/opt/homebrew/opt/pcre2/lib"
JSONC_LIB="/opt/homebrew/opt/json-c/lib"
OPENSSL_LIB="/opt/homebrew/opt/openssl@3/lib"

echo "→ Destination: $DEST"
mkdir -p "$BIN_DIR" "$LIB_DIR"

# ── Copy binaries ──────────────────────────────────────────────────────────────

echo "→ Copying binaries…"
cp -f "$CLAMAV_BIN/clamscan"   "$BIN_DIR/clamscan"
cp -f "$CLAMAV_BIN/freshclam"  "$BIN_DIR/freshclam"
chmod +x "$BIN_DIR/clamscan" "$BIN_DIR/freshclam"

# ── Copy dylibs ────────────────────────────────────────────────────────────────

echo "→ Copying dylibs…"
cp -f "$CLAMAV_LIB/libclamav.12.dylib"    "$LIB_DIR/libclamav.12.dylib"
cp -f "$CLAMAV_LIB/libclammspack.0.dylib" "$LIB_DIR/libclammspack.0.dylib"
cp -f "$CLAMAV_LIB/libfreshclam.4.dylib"  "$LIB_DIR/libfreshclam.4.dylib"
cp -f "$PCRE2_LIB/libpcre2-8.0.dylib"     "$LIB_DIR/libpcre2-8.0.dylib"
cp -f "$JSONC_LIB/libjson-c.5.dylib"      "$LIB_DIR/libjson-c.5.dylib"
cp -f "$OPENSSL_LIB/libssl.3.dylib"       "$LIB_DIR/libssl.3.dylib"
cp -f "$OPENSSL_LIB/libcrypto.3.dylib"    "$LIB_DIR/libcrypto.3.dylib"

# Make all copies writable so install_name_tool can modify them
chmod -R u+w "$BIN_DIR" "$LIB_DIR"

# ── Helper: fix a binary ───────────────────────────────────────────────────────

fix_binary() {
    local file="$1"
    echo "  fixing binary: $(basename "$file")"

    # Replace Cellar-specific LC_RPATH with @executable_path/../lib
    install_name_tool -rpath \
        "/opt/homebrew/Cellar/clamav/1.5.2/lib" \
        "@executable_path/../lib" \
        "$file" 2>/dev/null || true

    # Add the rpath in case it wasn't there under that exact path
    install_name_tool -add_rpath "@executable_path/../lib" "$file" 2>/dev/null || true

    # Fix absolute Homebrew paths → @executable_path/../lib/<name>
    install_name_tool \
        -change "$PCRE2_LIB/libpcre2-8.0.dylib"  "@executable_path/../lib/libpcre2-8.0.dylib"  \
        -change "$JSONC_LIB/libjson-c.5.dylib"    "@executable_path/../lib/libjson-c.5.dylib"   \
        -change "$OPENSSL_LIB/libssl.3.dylib"      "@executable_path/../lib/libssl.3.dylib"      \
        -change "$OPENSSL_LIB/libcrypto.3.dylib"   "@executable_path/../lib/libcrypto.3.dylib"   \
        "$file"
}

# ── Helper: fix a dylib ────────────────────────────────────────────────────────

fix_dylib() {
    local file="$1"
    local name
    name="$(basename "$file")"
    echo "  fixing dylib: $name"

    # Fix own install name
    install_name_tool -id "@rpath/$name" "$file"

    # Remove old Cellar rpath if present and add @loader_path
    install_name_tool -rpath \
        "/opt/homebrew/Cellar/clamav/1.5.2/lib" \
        "@loader_path" \
        "$file" 2>/dev/null || true
    install_name_tool -add_rpath "@loader_path" "$file" 2>/dev/null || true

    # Fix absolute Homebrew refs → @loader_path/<name>
    install_name_tool \
        -change "$CLAMAV_LIB/libclamav.12.dylib"    "@loader_path/libclamav.12.dylib"    \
        -change "$CLAMAV_LIB/libclammspack.0.dylib"  "@loader_path/libclammspack.0.dylib"  \
        -change "$CLAMAV_LIB/libfreshclam.4.dylib"   "@loader_path/libfreshclam.4.dylib"   \
        -change "$PCRE2_LIB/libpcre2-8.0.dylib"      "@loader_path/libpcre2-8.0.dylib"     \
        -change "$JSONC_LIB/libjson-c.5.dylib"       "@loader_path/libjson-c.5.dylib"      \
        -change "$OPENSSL_LIB/libssl.3.dylib"         "@loader_path/libssl.3.dylib"         \
        -change "$OPENSSL_LIB/libcrypto.3.dylib"      "@loader_path/libcrypto.3.dylib"      \
        "$file" 2>/dev/null || true
}

# ── Fix binaries ───────────────────────────────────────────────────────────────

echo "→ Fixing binaries…"
fix_binary "$BIN_DIR/clamscan"
fix_binary "$BIN_DIR/freshclam"

# ── Fix dylibs ─────────────────────────────────────────────────────────────────

echo "→ Fixing dylibs…"
for lib in "$LIB_DIR"/*.dylib; do
    fix_dylib "$lib"
done

# Special case: libssl has a Cellar-absolute ref to libcrypto (not via @rpath)
CELLAR_CRYPTO="$(otool -L "$LIB_DIR/libssl.3.dylib" \
    | awk '/libcrypto/ && /Cellar/ {print $1}')"
if [ -n "$CELLAR_CRYPTO" ]; then
    echo "  patching libssl → libcrypto Cellar ref: $CELLAR_CRYPTO"
    install_name_tool \
        -change "$CELLAR_CRYPTO" "@loader_path/libcrypto.3.dylib" \
        "$LIB_DIR/libssl.3.dylib"
fi

# ── Ad-hoc code signing ───────────────────────────────────────────────────────
# install_name_tool invalidates the original Apple signature; macOS will SIGKILL
# any binary with an invalid signature. Re-sign with a local ad-hoc identity so
# the binary runs in dev mode. Tauri replaces this with the real cert at bundle time.

echo "→ Ad-hoc signing…"
for lib in "$LIB_DIR"/*.dylib; do
    codesign --sign - --force "$lib" 2>/dev/null && echo "  ✓ $(basename "$lib")"
done
codesign --sign - --force "$BIN_DIR/clamscan"  2>/dev/null && echo "  ✓ clamscan"
codesign --sign - --force "$BIN_DIR/freshclam" 2>/dev/null && echo "  ✓ freshclam"

# ── Verify ─────────────────────────────────────────────────────────────────────

echo ""
echo "✓ Done. Checking for remaining Homebrew references:"
remaining=$(otool -L "$BIN_DIR"/clamscan "$BIN_DIR"/freshclam "$LIB_DIR"/*.dylib \
    | grep -E "/opt/homebrew|/usr/local/Cellar" || true)
if [ -n "$remaining" ]; then
    echo "⚠ Remaining references:"
    echo "$remaining"
else
    echo "  None found — all clean."
fi
