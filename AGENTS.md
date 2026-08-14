# Barestash CLI Development Guide

## Purpose

This public repository owns the standalone native `barestash` CLI. The CLI
receives and inspects webhook events through the Barestash API and supports
terminal, script, and AI-agent workflows without a Node.js runtime.

The command and output contract is documented in
[`docs/cli-design.md`](docs/cli-design.md).

## Repository boundaries

- Keep the repository root as a virtual Cargo workspace. All packages belong
  under `crates/`; the `barestash` binary belongs to `crates/cli`.
- Keep `crates/cli/src/main.rs` small. Organize CLI-only behavior by the
  `auth`, `endpoints`, `events`, and `tokens` features; each feature owns its
  clap inputs, use cases, transformations, and feature-specific presentation.
- Use the Rust 2018-style module layout: define parent modules as
  `src/<module>.rs` and child modules under `src/<module>/`; do not add
  `mod.rs` files.
- Keep wire contracts and portable protocol helpers in `crates/protocol`,
  reusable HTTP/SSE transport in `crates/client`, and config, credentials, and
  locking in `crates/local-state`. Dependencies remain acyclic: the CLI may
  depend on all three; the client may depend on the protocol; local-state stays
  independent of the protocol. Do not create crates for CLI-only architectural
  layers without an independent reuse or security boundary.
- Keep internal workspace crates unpublished. The supported compatibility
  surface is the `barestash` command, output, configuration, and credential
  contract rather than the Rust APIs between workspace crates.
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
- Keep sensitive-header redaction aligned with `crates/cli/src/events/headers.rs`
  and cover additions with tests.
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
