<div align="center">
  <img src="public/logo.png" width="180" alt="Burrow app icon">
  <h1>Burrow</h1>
  <p><strong>A native macOS cleaning and maintenance toolbox.</strong></p>
  <p>Keep your Mac clean, fast and easy to understand — from one friendly app. 🐾</p>
  <p>
    <img src="https://img.shields.io/badge/macOS-13%2B-111111?logo=apple&logoColor=white" alt="macOS 13+">
    <img src="https://img.shields.io/badge/Apple%20Silicon-arm64-111111?logo=apple&logoColor=white" alt="Apple Silicon arm64">
    <img src="https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white" alt="Tauri 2">
    <img src="https://img.shields.io/badge/Rust-native-000000?logo=rust&logoColor=white" alt="Rust">
    <img src="https://img.shields.io/badge/React-19-149ECA?logo=react&logoColor=white" alt="React 19">
    <a href="https://github.com/Luffytnes/Burrow/actions/workflows/ci.yml"><img src="https://github.com/Luffytnes/Burrow/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
    <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-22c55e.svg" alt="MIT License"></a>
  </p>
</div>

---

## ✨ What Burrow can do

- **📊 Dashboard** — follow CPU/GPU/SoC temperatures, fan speed, power, memory, network activity and running processes.
- **🧹 Clean** — remove caches, logs, crash reports, browser data, developer caches, old installers and duplicate files.
- **🛡️ Scan** — scan files with the bundled ClamAV engine, refresh malware definitions and quarantine suspicious files.
- **⬆️ Update** — discover updates from Homebrew casks, Sparkle feeds and the Mac App Store.
- **🗑️ Uninstall** — remove applications together with their associated preferences, caches and launch items.
- **🔎 Analyze** — explore disk usage, large files, developer artifacts and estimated storage growth.
- **⚡ Optimize** — manage Low Power Mode, memory maintenance and supported fan controls.
- **🔐 Private DNS** — configure encrypted DNS profiles and manage DNS/search-domain settings.

## 🍎 Compatibility

Burrow currently targets:

- **macOS 13 Ventura or later**
- **Apple Silicon only (`arm64`)**

Intel Macs are not supported. The bundled ClamAV, Mole, SMC and Touch ID resources are compiled exclusively for Apple Silicon, and the build system produces only an `aarch64-apple-darwin` target.

## 📦 Bundled components

The repository includes the initial arm64 resources required for a standalone build:

- [ClamAV](https://www.clamav.net) scanner and runtime libraries
- [Mole](https://github.com/tw93/Mole) maintenance CLI
- Burrow's SMC and Touch ID helpers, with their sources

Downloaded ClamAV definition databases, build outputs and local caches are intentionally excluded from Git.
The exact ClamAV 1.5.4 corresponding source archive is kept in
[`third_party/sources`](third_party/sources) alongside its SHA-256 checksum.
Versions, checksums and exact third-party license texts are tracked in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
Ongoing changes are summarized in [CHANGELOG.md](CHANGELOG.md).

ClamAV malware definitions can be refreshed from Burrow. Executable and library updates are delivered with a new Burrow release rather than modifying the application bundle in place.

## 🔑 macOS permissions

Some features request additional permissions only when macOS requires them:

- **Full Disk Access** — needed to inspect or clean protected locations.
- **Administrator approval** — needed for selected system, power, DNS and hardware operations.
- **Touch ID or password** — may be requested by macOS for privileged actions.

Always review an operation before confirming a destructive or privileged action.

## 🛠️ Development

### Prerequisites

- [Rust](https://rustup.rs), stable toolchain
- [Node.js](https://nodejs.org) 20 or later
- Xcode Command Line Tools

### Run locally

```bash
npm install
npm run tauri dev
```

### Useful checks

```bash
npm run typecheck
npm run lint
npm run format:check
# or run all three:
npm run check

cd src-tauri
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test

cd ..
scripts/verify_bundled_resources.sh
```

## 🚀 Build a release bundle

```bash
npm run tauri build -- --target aarch64-apple-darwin
```

Generated application and disk-image bundles are written under `src-tauri/target/release/bundle`.
The release checklist and artifact validation steps are documented in [RELEASING.md](RELEASING.md).

## 🗂️ Project layout

```text
src/                   React frontend (pages, components, hooks, i18n)
src-tauri/src/         Rust backend and Tauri commands
  duplicates.rs        Duplicate-file scanner
  gpu.rs               GPU information
  guard.rs             Safety validation for sensitive operations
  ior.rs               IOReport and SMC bindings
  lib.rs               Application commands and orchestration
src-tauri/resources/   Bundled Apple Silicon resources
scripts/               Resource and packaging helpers
```

## 🔒 Security

Burrow performs sensitive maintenance operations, so its security model treats frontend input as untrusted. The project uses backend validation, restricted Tauri capabilities and a Content Security Policy, with additional hardening tracked as development continues.

Please do not use real personal data when developing or testing destructive operations. Use isolated temporary fixtures instead.

Please report vulnerabilities privately as described in [SECURITY.md](SECURITY.md).
Local data handling and network-backed features are described in [PRIVACY.md](PRIVACY.md).

## 🤝 Contributing

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request, and keep security-sensitive changes small and testable.

## 📄 License

Burrow's own source code is available under the [MIT License](LICENSE). Bundled third-party components keep their respective licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for details.

---

<div align="center">
  Made for Apple Silicon with Rust, Tauri and a very determined little mole. 🐹
</div>
