# Contributing to Burrow

Thanks for helping improve Burrow. 🐾

## Before you start

- Target Apple Silicon and macOS 13 or later.
- Keep pull requests focused and explain user-visible or security-sensitive
  behavior changes.
- Open an issue first for large product changes. A maintainer may close a
  proposal that does not fit Burrow's scope or safety model.
- Never commit secrets, signing certificates, malware samples, personal files,
  downloaded virus databases or generated build output.

## Local checks

Install Node.js 22, the stable Rust toolchain and Xcode Command Line Tools,
then run:

```bash
npm ci
npm run check
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
scripts/verify_bundled_resources.sh
```

To regenerate the macOS application icon from the original Burrow artwork,
run `npm run icon:macos`. The generator keeps the artwork inside the standard
macOS visual bounds and writes both the reviewable PNG and bundled ICNS file.

## Safety requirements

- Treat every Tauri command argument as attacker-controlled.
- Use typed arguments and `Command::arg`; do not pass frontend text to a shell.
- Keep destructive operations behind backend validation and a recent scan-issued
  path grant.
- Refuse ambiguous updates. Verify the downloaded application's bundle ID,
  signing identifier and Team ID before replacing an installed app.
- Add regression tests for every fixed security issue.

## Licensing

Contributions are accepted under the repository's GNU GPL version 3 license.
By submitting a contribution, you agree that it may be distributed under
`GPL-3.0-only`. New third-party code or binaries must have a compatible,
documented license. Update
`THIRD_PARTY_NOTICES.md` and include the exact license text, version and
checksums when applicable.

Submitting a contribution does not guarantee that it will be merged. Burrow's
maintainer makes the final decision based on product scope, safety,
maintainability and licensing.
