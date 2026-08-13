# Barestash CLI Development Guide

## Purpose

This public repository owns the standalone native `barestash` CLI. The CLI
receives and inspects webhook events through the Barestash API and supports
terminal, script, and AI-agent workflows without a Node.js runtime.

The command and output contract is documented in
[`docs/cli-design.md`](docs/cli-design.md).

## Repository boundaries

- Keep the Cargo package and `barestash` binary at the repository root.
- Keep `src/main.rs` small; parsing, application behavior, infrastructure, and
  presentation remain separate modules.
- Use the Rust 2018-style module layout: define parent modules as
  `src/<module>.rs` and child modules under `src/<module>/`; do not add
  `mod.rs` files.
- Protocol types in `src/protocol.rs` describe only public API contracts and
  portable helpers needed by this CLI.
- Do not add server implementation, deployment configuration, private
  operational documentation, or backend-only contracts.

## CLI contract

- Preserve `barestash {resource} {action}`.
- Human-readable output is the default. Use explicit `--json`; `events stream`
  writes JSON Lines only to stdout.
- Keep diagnostics on stderr whenever stdout is reserved for structured data.
- Resolve endpoints as `--endpoint`, `BARESTASH_ENDPOINT`, then local config.
- Resolve tokens as non-empty `BARESTASH_TOKEN`, stored credential, then the
  legacy TOML config token.
- Keep expected errors actionable and free of stack traces.

## Security

- Never print or commit raw tokens, refresh credentials, endpoint secrets,
  cookies, `Authorization`, or `x-barestash-secret` except where a one-time
  creation result explicitly owns the secret on stdout.
- Keep sensitive-header redaction aligned with `src/domain.rs` and cover
  additions with tests.
- Treat body content as potentially sensitive even though the CLI does not
  redact it by default.
- Preserve API URL and same-origin redirect validation for authenticated
  requests.
- Preserve atomic credential writes, user-only permissions, logged-out marker
  semantics, and the credential/config lock.

## Development and verification

- Use `just` as the documented command surface.
- Run `just check` for source or configuration changes.
- Run `just package` whenever manifest or release files change.
- Run `just ci` before a release.
- Update `README.md` and `docs/cli-design.md` with user-visible behavior.
- Update `docs/release.md` when artifact verification or publishing changes.
