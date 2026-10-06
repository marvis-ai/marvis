# Security Policy

Marvis handles sensitive data — screen frames, microphone and system audio,
and provider API keys — all stored locally under `~/.marvis`. We take
security reports seriously and appreciate responsible disclosure.

## Supported versions

Security fixes ship on the latest release only.

| Version | Supported |
| --- | --- |
| Latest release | Yes |
| Older releases | No |

## Reporting a vulnerability

**Please do not open a public issue, pull request, or discussion for
security reports.**

Email <support@getmarvis.com> with:

- A description of the issue and its potential impact
- The affected version(s) and platform(s) (macOS / Windows / Linux)
- Steps to reproduce or a proof of concept, if available

We aim to acknowledge reports within 3 business days and to share a triage
decision within a week. If the report is accepted, we will work on a fix,
coordinate a release, and credit you in the release notes unless you prefer
to remain anonymous.

## What we're especially interested in

- API keys or user data leaking to logs, the webview, or the network
- Escalation through the `invoke()` command surface, e.g. calls that bypass
  the server-side gate checks
- Path traversal or code execution via model downloads, deep links
  (`marvis://`), or the capture pipeline
- Screen or audio data persisting outside `~/.marvis` or outliving its
  documented retention

## Out of scope

- Vulnerabilities in third-party providers, models, or dependencies —
  report those upstream
- Issues that require physical access to an unlocked machine or an
  already-compromised OS
- Findings from automated scanners without a demonstrated impact
