<div align="center">
  <img src="public/logo.png" width="180" alt="Burrow app icon">

  # Burrow

  **A native macOS cleaning and maintenance toolbox.**

  Keep your Mac clean, fast and easy to understand — from one friendly app. 🐾

  ![macOS](https://img.shields.io/badge/macOS-12%2B-111111?logo=apple&logoColor=white)
  ![Apple Silicon](https://img.shields.io/badge/Apple%20Silicon-arm64-111111?logo=apple&logoColor=white)
  ![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)
  ![Rust](https://img.shields.io/badge/Rust-native-000000?logo=rust&logoColor=white)
  ![React](https://img.shields.io/badge/React-19-149ECA?logo=react&logoColor=white)
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

- **macOS 12.0 or later**
- **Apple Silicon only (`arm64`)**

Intel Macs are not currently supported because the bundled ClamAV, Mole and SMC resources are built for Apple Silicon.

## 📦 Bundled components

The repository includes the initial arm64 resources required for a standalone build:

- [ClamAV](https://www.clamav.net) scanner and runtime libraries
- [Mole](https://github.com/tw93/Mole) maintenance CLI
- Burrow's SMC helper and its Swift source

Downloaded ClamAV definition databases, build outputs and local caches are intentionally excluded from Git.

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

cd src-tauri
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## 🚀 Build a release bundle

```bash
npm run tauri build
```

Generated application and disk-image bundles are written under `src-tauri/target/release/bundle`.

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

---

<div align="center">
  Made for Apple Silicon with Rust, Tauri and a very determined little mole. 🐹
</div>
