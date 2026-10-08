# Security

## Intended use

codeenv is for a personal server or a trusted team. Every authenticated user
can run a full shell under the same Unix account. Terminal ownership by email
is not an operating-system security boundary. The explorer's `root` setting
does not restrict shell commands.

Use Cloudflare Access, configure allowed hosts, and run under a dedicated
account without passwordless sudo. Local `insecure` mode is only for loopback
access during development. See the [deployment guide](docs/deployment.md).

## Report a vulnerability

Use [GitHub private vulnerability reporting](https://github.com/leyoshe/codeenv/security/advisories/new).
Include the affected version, reproduction steps, likely impact, and a minimal
example if possible. Do not open a public issue containing an exploit or secrets.

If private reporting is unavailable, open an issue asking maintainers to enable
it, without describing the vulnerability. Never post Access tokens, cookies,
or production credentials.

This is a young project without a guaranteed response time or a formal security
audit. Fixes target the latest release; older versions do not have a separate
maintenance commitment.

## Test credentials

The RSA key pair in `src/testdata/` is public test data used by the JWT tests.
It must never be used for authentication outside tests. It is not a production
Cloudflare Access credential.
