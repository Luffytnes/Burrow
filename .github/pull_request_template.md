## What changed

<!-- Describe the user-visible and technical changes. -->

## Why

<!-- Link the issue or explain the problem. -->

## Validation

- [ ] `npm run check`
- [ ] `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`
- [ ] `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml`
- [ ] `scripts/verify_bundled_resources.sh`
- [ ] I manually tested the affected macOS flow where practical

## Security and licensing

- [ ] No secret, personal data, signing material or generated database is included
- [ ] New frontend inputs are validated by the Rust backend
- [ ] New dependencies or bundled binaries include version, checksum and license information
