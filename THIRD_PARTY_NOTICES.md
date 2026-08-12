# Third-party notices

Burrow is released under the MIT License. The repository also contains the
following independently licensed Apple Silicon components. Their licenses
apply to those components and are not replaced by Burrow's MIT License.

## ClamAV 1.5.4

- Project: <https://github.com/Cisco-Talos/clamav>
- Source release: <https://github.com/Cisco-Talos/clamav/releases/tag/clamav-1.5.4>
- Corresponding source archive: `third_party/sources/clamav-1.5.4.tar.gz`
- License: GNU General Public License v2.0 or later, with the OpenSSL exception
- Bundled files: `src-tauri/resources/clamav/bin` and
  `src-tauri/resources/clamav/lib`
- Rebuild helper: `scripts/bundle_clamav.sh`

The exact license texts for ClamAV and its bundled runtime dependencies are in
`src-tauri/resources/clamav/licenses`. Checksums are recorded in
`src-tauri/resources/clamav/SHA256SUMS`. The exact corresponding ClamAV source
archive distributed with the binaries is recorded in
`third_party/sources/SHA256SUMS`.

Bundled ClamAV runtime dependencies:

- PCRE2 — BSD 3-Clause
- json-c — MIT
- OpenSSL — Apache License 2.0

## Mole 1.39.1

- Project: <https://github.com/tw93/Mole>
- Source tag: <https://github.com/tw93/Mole/tree/V1.39.1>
- License: MIT
- Bundled files: `src-tauri/resources/mole/bin`

The upstream license is included at `src-tauri/resources/mole/LICENSE`.

## Burrow helpers

`burrow-smc` and `burrow-touchid` are Burrow components released under the
repository's MIT License. Their sources are included alongside the resources.

This notice is informational and is not legal advice. If a packaged build adds
or replaces a binary dependency, update this document, the corresponding
license text, version marker and checksum manifest in the same change.
