# Burrow

**macOS cleaning & maintenance toolbox** — a native desktop app built with [Tauri 2](https://tauri.app), React 19 and Rust.

Burrow keeps your Mac fast and tidy from a single window: system cleanup, malware scanning, app updates, uninstaller, disk analysis, DNS configuration and live hardware monitoring.

## Features

- **Dashboard** — live CPU/GPU/SoC temperatures, fan RPM, power draw (via IOReport/SMC), memory pressure, network rates, process manager.
- **Clean** — user/system caches, logs, crash reports, browser caches, npm/yarn/Homebrew caches, Xcode DerivedData, iOS backups, old installers, duplicate finder.
- **Scan** — on-demand malware scanning powered by a bundled [ClamAV](https://www.clamav.net), with signature updates and a quarantine (isolate / restore / delete).
- **Update** — detects outdated apps via Homebrew casks, Sparkle feeds and the Mac App Store, and updates them in place.
- **Uninstall** — removes apps together with their leftovers (preferences, caches, launch agents), and suggests popular apps to install via Homebrew.
- **Analyze** — disk usage breakdown, large files, dev caches, project artifacts, disk-full forecast.
- **Optimize** — memory purge, Low Power Mode toggle, fan control (auto / max / custom) via a bundled SMC helper.
- **DNS** — one-click encrypted DNS (DoH profiles) for Mullvad, Quad9, AdGuard, Cloudflare or LibreDNS, plus manual DNS and search-domain management.

## Requirements

- macOS 12.0 or later on Apple Silicon (arm64)
- [Rust](https://rustup.rs) (stable) and [Node.js](https://nodejs.org) ≥ 20 to build from source

Some features request extra permissions the first time you use them, and explain why:

- **Full Disk Access** — required to measure and clean caches outside the sandboxed areas.
- **Administrator (Touch ID / password)** — required for fan control, Low Power Mode and DNS changes. Fan control installs a narrowly-scoped `sudoers.d` rule so it can run without prompting each time.

## Development

```bash
npm install
npm run tauri dev      # run the app with hot reload
```

Useful scripts:

```bash
npm run typecheck      # TypeScript check
npm run lint           # ESLint
npm run format         # Prettier
cargo test             # Rust unit tests (from src-tauri/)
```

## Build a release bundle

```bash
npm run tauri build    # produces Burrow.app / .dmg in src-tauri/target/release/bundle
```

## Project layout

```
src/                 React frontend (pages, components, hooks, i18n)
src-tauri/src/       Rust backend
  lib.rs             Tauri commands (system, clean, scan, updates, DNS…)
  duplicates.rs      Duplicate-file scanner
  gpu.rs             GPU information
  guard.rs           Path/PID validation for destructive commands
  ior.rs             IOReport bindings (temperatures, power)
src-tauri/resources/ Bundled arm64 helpers: mole CLI, ClamAV, burrow-smc
scripts/             Packaging helpers (ClamAV bundling)
```

## Security notes

- The WebView runs with a strict Content-Security-Policy; the only remote origins allowed are `formulae.brew.sh` (cask metadata) and Google favicons.
- Destructive commands (`delete_path`, `move_to_trash`, quarantine, `kill_process`) validate their input against an allowlist of user-data locations — system directories and protected roots are always refused (see `src-tauri/src/guard.rs`).
- Shell access from the frontend is limited by Tauri capabilities to `df -k` and opening URLs.
