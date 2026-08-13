# Changelog

All notable changes to Burrow are documented here. Burrow is pre-1.0 software,
so security and behavior changes may still be substantial between versions.

## 0.2.1 — 2026-08-13

### Fixed

- Replace the legacy transparent app-icon canvas with an opaque, full-bleed
  macOS icon so macOS 26 no longer adds a grey compatibility enclosure.
- Remove an intermediate `.burrow-thinned-*.app` copy if the final
  universal-binary replacement cannot be installed.

### Changed

- Open Burrow on Smart Scan by default.
- Replace the README hero image with the Smart Scan interface.

## 0.2.0 — 2026-08-13

### Added

- Add Smart Scan, combining recoverable cleanup, a focused ClamAV security
  scan and system maintenance recommendations in one guided workflow.
- Add an interactive disk-usage treemap with directory drill-down.
- Add a live menu-bar widget for CPU, memory, disk, temperature and GPU status,
  with shortcuts to Smart Scan and the activity journal.
- Add a private, size-bounded, centralized activity journal for scans,
  cleanups, app operations and maintenance actions.
- Add working Apple Silicon thinning for universal application bundles,
  including per-bundle discovery, verification and recovery of the original
  application from the Trash.

### Security

- Require backend-issued, inode-bound grants for destructive file actions.
- Restrict updates, DNS profiles, Homebrew operations, image previews and
  privileged commands to typed and revalidated backend inputs.
- Harden ClamAV scanning and disk browsing against symbolic links, path
  replacement, unbounded output and concurrent process abuse.
- Enforce backend-managed Touch ID protection for application removal.
- Make user-facing cleanup and removal operations recoverable through the
  macOS Trash, and remove irreversible Trash, snapshot and purge actions.

### Changed

- Target macOS 13 or later on Apple Silicon only.
- Prepare reproducible arm64 application and DMG artifacts with checksums and
  CycloneDX SBOMs.
- Add corresponding ClamAV source, third-party notices and automated resource
  integrity checks.
- Add public contribution, security, issue and repository-maintenance guidance.
- Add a production frontend build to continuous integration.

## 0.1.0 — 2026-08-12

- Early source preview. No installable end-user artifact was published.
