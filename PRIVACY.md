# Privacy

Burrow is a local macOS utility. It has no Burrow account system and includes
no first-party analytics, advertising SDK or telemetry service.

## Local data

Scanning, cleanup analysis, hardware monitoring and application inventory are
processed on the Mac. Burrow stores only the local state needed for features
such as settings, quarantine metadata, malware definitions and disk-history
estimates. Removing the application does not automatically remove quarantined
files or its application-data directory.

Some screens display local filenames and application names. Debug logs or
screenshots may therefore contain personal information; sanitize them before
sharing a bug report.

## Network access

Network-backed features make direct requests to their relevant providers:

- Homebrew metadata and analytics endpoints for the application catalogue and
  update discovery;
- update feeds declared by installed applications, Apple software lookup
  services and selected download hosts when checking or applying updates;
- ClamAV infrastructure when refreshing malware definitions.

These services receive ordinary connection metadata such as the IP address and
user agent. Their own privacy policies apply. Burrow does not proxy those
requests through a Burrow-operated server.

Encrypted-DNS profiles are generated from a fixed backend catalogue and opened
locally for macOS to install. DNS traffic then follows the provider selected by
the user.

## Security reports

Do not attach personal files, malware samples or unsanitized logs to public
issues. Follow [SECURITY.md](SECURITY.md) for private vulnerability reports.
