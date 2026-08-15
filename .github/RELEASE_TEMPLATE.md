# Burrow vX.Y.Z — macOS Apple Silicon

> **Platform:** macOS 13 Ventura or later · Apple Silicon (arm64) only
> **License:** GPL-3.0-only · [Third-party notices](THIRD_PARTY_NOTICES.md)

## What's changed

<!-- List notable changes, fixes, and improvements. -->

- ...

## Security

<!-- Describe any security fixes or hardening work. -->

- ...

## Bundled components

| Component      | Version | License |
| -------------- | ------- | ------- |
| ClamAV         | 1.5.4   | GPLv2+  |
| Mole           | 1.48.1  | GPLv3   |
| burrow-smc     | —       | GPLv3   |
| burrow-touchid | —       | GPLv3   |

All bundled Mach-O binaries are verified as arm64. SHA-256 checksums are
published with the artifacts.

## Known limitations

<!-- List residual risks or design trade-offs that remain open. -->

- `cask_api()` fetches live cask metadata from `formulae.brew.sh` on startup, so
  the download URL is not pinned. The backend still validates the downloaded
  application bundle before installation.

## Requirements

- macOS 13 Ventura or later (Apple Silicon)
- **Optional:** Full Disk Access (for system cache scans), Xcode Command Line Tools
  (for Simulator runtime management), Homebrew (for cask install/update)

## Installation

1. Download the `.dmg` from the assets below.
2. Open the DMG and drag **Burrow.app** to `/Applications`.

## SHA-256 checksums

<!-- Paste the content of the SHA256SUMS file attached to this release. -->

```
<paste SHA256SUMS here>
```

## Action required before publishing

> **Note:** This template is for human review. Remove this block before publishing.
>
> - [ ] Fill in "What's changed" with meaningful entries.
> - [ ] Confirm "Bundled components" versions match the current build.
> - [ ] Paste the SHA-256 checksums from the CI artifacts.
> - [ ] Attach SBOM files (`*.cdx.json`) from the CI artifacts.
