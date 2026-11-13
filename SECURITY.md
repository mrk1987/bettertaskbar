# Security Policy

## Supported versions

Only the latest release receives security fixes.

## Reporting a vulnerability

Please **do not** open a public issue for security problems.

Use GitHub's private vulnerability reporting on the repository's **Security**
tab (**Report a vulnerability**), with:

- A description of the issue and its impact.
- Steps to reproduce, including the Windows version and build.
- Any suggested fix or mitigation.

You can expect an acknowledgement within a few days. Once a fix is available,
a release and advisory will be published and you will be credited unless you
prefer otherwise.

## Scope

BetterTaskbar is a local desktop application. It collects no telemetry.

What it does connect to, and what it stores:

- **Weather:** HTTPS requests to MET Norway and Yr, and to Open-Meteo when you
  search for a place. Certificates are verified.
- **Calendar and mail:** the CalDAV and IMAP servers you configure. Certificates
  are verified, except for `localhost` and private addresses (loopback, RFC 1918,
  100.64.0.0/10, link-local), where local bridges use self-signed certificates.
- **Credentials:** stored as plain text in `%APPDATA%\BetterTaskbar\widgets.json`,
  readable by anything running as your user. Mail is opened read-only.
- **Logs:** `%APPDATA%\BetterTaskbar\widgets.log` holds error messages. It must
  never contain credentials; report it if it does.

Vulnerabilities in dependencies such as `eframe`/`egui`, `wgpu`, `ureq` or
`native-tls` should be reported upstream; we will update dependencies as
advisories are published.
