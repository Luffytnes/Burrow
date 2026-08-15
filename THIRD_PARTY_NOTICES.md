# Third-party notices

Burrow is released under the GNU General Public License version 3 only. The
repository also contains the following independently licensed Apple Silicon
components. Their licenses continue to apply to those components.

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

## Mole 1.48.1

- Project: <https://github.com/tw93/Mole>
- Source tag: <https://github.com/tw93/Mole/tree/V1.48.1>
- Corresponding source archive: `third_party/sources/mole-1.48.1.tar.gz`
- License: GNU General Public License version 3
- Bundled files: `src-tauri/resources/mole/bin` and
  `src-tauri/resources/mole/libexec`
- Architecture: Apple Silicon (`arm64`)

The upstream license is included at `src-tauri/resources/mole/LICENSE`.
Burrow's `bin/mo` and `bin/mole` files are packaging wrappers which invoke the
pinned upstream engine from the application resources. Burrow adds a typed
Rust boundary in front of Mole: the frontend cannot provide arbitrary CLI
arguments, and recoverable operations force Mole's Trash mode.

## Burrow helpers

`burrow-smc` and `burrow-touchid` are Burrow components released under the
repository's GPL-3.0-only license. Their sources are included alongside the
resources.

## DNS provider identifiers

The provider artwork in `public/dns` is used solely to identify the respective
DNS services in Burrow. Provider names, logos and trademarks remain the
property of their respective owners and are not covered by Burrow's GPL
license.

New provider artwork and official service information for 0.2.3:

- FDN: <https://www.fdn.fr/actions/dns/>
- DNS4EU: <https://joindns4.eu/for-public>
- DNS.SB: <https://dns.sb/>
- OpenNIC: <https://opennic.org/>

This notice is informational and is not legal advice. If a packaged build adds
or replaces a binary dependency, update this document, the corresponding
license text, version marker and checksum manifest in the same change.
