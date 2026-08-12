# Releasing Burrow

Burrow releases target Apple Silicon only (macOS 13 Ventura or later) using
the `aarch64-apple-darwin` target.

## Checklist

1. Update the version consistently in `package.json`, `src-tauri/Cargo.toml` and
   `src-tauri/tauri.conf.json`.
2. Review dependency updates and run both npm and Rust security audits.
3. If a bundled component changed, update its version marker, checksum manifest,
   license text and `THIRD_PARTY_NOTICES.md` in the same commit.
4. Run the complete local validation:

   ```bash
   npm ci
   npm run check
   npm audit --audit-level=high
   cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
   cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
   cargo test --manifest-path src-tauri/Cargo.toml
   cargo audit --file src-tauri/Cargo.lock
   cargo deny --manifest-path src-tauri/Cargo.toml check advisories licenses sources bans
   scripts/verify_bundled_resources.sh
   npm run tauri build -- --target aarch64-apple-darwin
   ```

5. Confirm the final `.app` executable and every bundled Mach-O resource are
   arm64. The release-artifact workflow performs this check automatically on
   every Mach-O file in the bundle. For local builds run:
   ```bash
   file src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Burrow.app/Contents/MacOS/*
   ```
   Reject any bundle that contains an `x86_64` or `universal` binary.
6. Test the build on a clean Apple Silicon Mac, including first launch,
   permission prompts, ClamAV definition updates, quarantine/restore, uninstall,
   app updates and rollback behavior.
7. Publish release notes that call out security fixes, destructive behavior
   changes and third-party binary updates. Attach the CI-generated SBOM.

The workflow prepares reviewed artifacts but never publishes or edits a GitHub
release automatically.
