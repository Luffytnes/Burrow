# Changelog

All notable changes to Burrow are documented here. Burrow is pre-1.0 software,
so security and behavior changes may still be substantial between versions.

## 0.2.3 — 2026-08-14

### Added

- Replace the automatic home-folder explorer with an actionable priority
  center for safe cleanup, review items, large files, duplicates and universal
  binaries.
- Move the storage treemap to **Analyze > Storage** and require an explicit
  folder selection before scanning.
- Add compatibility feedback for applications whose original code signature
  is invalid or missing.
- Add FDN, DNS4EU, DNS.SB and OpenNIC to the private DNS catalog, including all
  five official DNS4EU filtering profiles and encrypted DNS profiles wherever
  the provider publishes a stable DoH endpoint.

### Fixed

- Preserve publisher signatures while thinning universal applications instead
  of re-signing nested components ad hoc with `codesign --deep`.
- Verify the source, temporary copy and prepared application before moving the
  original to the Trash, and abort cleanly when any verification fails.
- Cover nested application bundles with an automated signature-preservation
  regression test.

## 0.2.2 — 2026-08-13

### Added

- Let users select individual cleanup categories and maintenance tasks before
  applying Smart Scan recommendations.
- Keep deferred Smart Scan actions available after a partial run and visibly
  mark completed actions.
- Add a reproducible macOS icon generator and a continuous-integration compile
  check for it.

### Fixed

- Give the macOS application icon the expected transparent canvas and visual
  bounds so it no longer appears as an oversized red square in the Dock.
- Keep the Smart Scan progress indicator visibly rotating throughout analysis
  and selected-action execution, including in reduced-motion mode at a slower
  speed.
- Clarify that the Smart Scan security pass is analysis-only and never applies
  automatic remediation.

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
