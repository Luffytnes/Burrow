# Corresponding third-party source

This directory accompanies GPL-licensed executable components committed to the
repository and included in Burrow artifacts.

`clamav-1.5.4.tar.gz` is the exact upstream ClamAV 1.5.4 source release archive.
Its SHA-256 digest is recorded in `SHA256SUMS`. Burrow's resource assembly steps
are documented in `scripts/bundle_clamav.sh`.

When ClamAV is updated, replace the executable resources, corresponding source
archive, version marker, license notices and every relevant checksum in the same
change.
