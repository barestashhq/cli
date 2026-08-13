# Security Policy

This policy covers the native Barestash CLI distributed through GitHub
Releases.

## Supported versions

| Version | Supported |
| --- | --- |
| Latest `main` | Yes |
| Latest published release | Yes |
| Older commits or releases | No |

Until a stable release series exists, security fixes land on `main` and are
included in the next release. Older snapshots are not backported by default.

## Reporting a vulnerability

Do not report vulnerabilities through public issues, discussions, or pull
requests. Use GitHub private vulnerability reporting:

<https://github.com/barestashhq/cli/security/advisories/new>

Include the affected version or commit, impact, reproduction steps, and a
minimal proof of concept when possible. Do not include live Personal Access
Tokens, refresh credentials, endpoint secrets, private endpoint URLs, or real
captured webhook payloads.

There is currently no bug bounty program. Please keep details private until a
fix is available or a disclosure plan is agreed.

## Scope

In-scope issues include:

- Theft or disclosure of credentials stored or handled by the CLI
- Failure to redact sensitive request headers in terminal or structured output
- Authenticated requests sent to an unsafe destination through API URL or
  redirect handling
- Command parsing or event processing vulnerabilities reachable through
  untrusted input
- Release artifact, checksum, or update-path issues that can execute unintended
  code

Server-side authorization, storage, hosted-service availability, and
infrastructure are outside this repository's implementation scope. If an issue
crosses the CLI/API boundary or you are unsure where it belongs, report it
privately here and maintainers will route it.

## Local security guidance

- Prefer the operating system credential store.
- Treat plaintext fallback warnings seriously and protect the referenced file.
- Use scoped, expiring tokens and revoke leaked credentials promptly.
- Keep `.env`, the local TOML config, the credential JSON fallback, captured
  payloads, and debug logs out of repositories and support requests.
- Use `--allow-insecure-api-url` only for an API host you intentionally trust.
