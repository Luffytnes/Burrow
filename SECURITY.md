# Security policy

## Supported version

Security fixes currently target the latest revision of the `main` branch.
Burrow is pre-1.0 software; older builds may stop receiving fixes without a
separate deprecation period.

## Reporting a vulnerability

Please do not open a public issue for a suspected vulnerability. Use GitHub's
private vulnerability reporting feature on the repository's **Security** tab.
Include the affected version or commit, reproduction steps, impact, and any
suggested mitigation.

Please avoid accessing data that is not yours, disrupting other systems, or
publishing details before a fix is available. A report will be acknowledged as
soon as practical, then updated when its scope and remediation are understood.

## Security model

Burrow performs privileged and destructive maintenance operations. Backend
commands therefore treat all webview input as untrusted, validate paths and
arguments, and require recent scan-issued grants for sensitive file actions.
macOS authorization prompts remain the final authority for privileged changes.
