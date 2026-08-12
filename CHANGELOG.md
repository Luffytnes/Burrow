# Changelog

All notable changes to Burrow are documented here. Burrow is pre-1.0 software,
so security and behavior changes may still be substantial between versions.

## Unreleased

### Security

- Require backend-issued, inode-bound grants for destructive file actions.
- Restrict updates, DNS profiles, Homebrew operations, image previews and
  privileged commands to typed and revalidated backend inputs.
- Harden ClamAV scanning and disk browsing against symbolic links, path
  replacement, unbounded output and concurrent process abuse.
- Enforce backend-managed Touch ID protection for application removal.

### Changed

- Target macOS 13 or later on Apple Silicon only.
- Prepare reproducible arm64 application and DMG artifacts with checksums and
  CycloneDX SBOMs.
- Add corresponding ClamAV source, third-party notices and automated resource
  integrity checks.
- Add public contribution, security, issue and repository-maintenance guidance.

## 0.1.0 — 2026-08-12

- Early source preview. No installable end-user artifact was published.
