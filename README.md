# Barestash CLI

**A native, headless stash for incoming requests.**

Barestash receives webhooks, preserves the raw HTTP request, and lets terminal
users, scripts, and AI agents inspect or stream each event. The CLI is a single
Rust binary and does not require Node.js.

```text
External service → Barestash endpoint → Raw request stashed → CLI / JSONL stream
```

## Installation

Download the archive for your platform from
[GitHub Releases](https://github.com/barestashhq/cli/releases), verify the
adjacent `.sha256` file, and put `barestash` (or `barestash.exe`) on `PATH`.
Release assets use these target names:

- `aarch64-apple-darwin` and `x86_64-apple-darwin`
- `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`
- `x86_64-pc-windows-msvc`

You can also install the current source with a Rust 1.88+ toolchain:

```bash
cargo install --locked --git https://github.com/barestashhq/cli barestash
```

A Homebrew tap is planned; until it is published, use the macOS GitHub Release
archive. The former npm package is not needed by the native CLI.

## Quick start

Create an unauthenticated temporary endpoint, follow it, and send a request:

```bash
barestash endpoints create --temporary --set-default
barestash events tail

curl -X POST https://ingest.example.com/ep_abc123/test \
  -H 'content-type: application/json' \
  -d '{"hello":"world"}'
```

Commands follow `barestash {resource} {action}`. Start with:

```bash
barestash --help
barestash events --help
```

## Authentication

Use Device Authorization for an interactive CLI session:

```bash
barestash auth login
barestash auth status
barestash auth logout --revoke
```

The login command prints a verification URL and one-time code to stderr, opens
the verification page when possible, polls at the server-selected interval,
and securely stores the refreshable session. Access tokens are refreshed and
rotated automatically near expiration.

To validate and store an existing Personal Access Token (PAT), pass it through
stdin so it is not copied into process arguments:

```bash
printf '%s' "$BARESTASH_TOKEN" | barestash auth login --with-token
```

Credentials use macOS Keychain, Windows Credential Manager, or Linux Secret
Service by default. Existing `barestash`/`default` credentials from the former
CLI are reused, including legacy Linux Secret Service attributes and the former
Windows Credential Manager target. If the OS store is unavailable, the CLI
explicitly warns before using a user-only plaintext fallback.
`--insecure-storage` selects that fallback deliberately. On Unix the file is
mode `0600`; on Windows it receives a current-user-only ACL. Credential and
config mutations share an OS file lock.

Authentication token resolution is:

1. non-empty `BARESTASH_TOKEN`
2. stored credential
3. legacy `token` in the TOML config (final fallback)

## Endpoints

```bash
barestash endpoints create [--private | --temporary] [--name NAME] [--set-default] [--json]
barestash endpoints list [--json]
barestash endpoints show EP_ID [--json]
barestash endpoints delete EP_ID [--yes]

barestash endpoints secrets create [--endpoint EP_ID] [--json]
barestash endpoints secrets list [--endpoint EP_ID] [--json]
barestash endpoints secrets revoke SECRET_ID [--endpoint EP_ID] [--yes]
```

Private endpoints require authentication. Temporary endpoints can be created
and read through REST/polling without authentication, but do not support the
live SSE stream. An endpoint secret is shown only when it is created.

Commands accepting `--endpoint` select it in this order:

1. `--endpoint EP_ID`
2. `BARESTASH_ENDPOINT`
3. `default_endpoint` in the local TOML config

## Events

```bash
barestash events list [--endpoint EP_ID] [--limit N] [--json]
barestash events latest [--endpoint EP_ID] [--json]
barestash events show EVENT_ID [--json]
barestash events tail [--endpoint EP_ID] [--last N] [--headers] [--body]
barestash events stream [--endpoint EP_ID]
```

Sensitive request headers are redacted in both human and machine output.
Captured bodies are not redacted: treat output as potentially sensitive.

### Polling tail

`events tail` appends new events to stdout and accepts a unit-bearing polling
interval such as `500ms`, `2s`, or `1m`:

```bash
barestash events tail --last 10 --headers --body --poll-interval 500ms
```

For the existing simple screen-updating view, use an interactive terminal:

```bash
barestash events tail --view
```

The view shows endpoint status, received count, last-event time, and recent
requests. It adapts to terminal width/height without raw mode, alternate screen,
or a full TUI. It cannot be combined with `--headers` or `--body`.

### SSE stream and JSONL

`events stream` writes exactly one compact JSON object per event to stdout:

```bash
barestash events stream --endpoint ep_abc123 | jq .
```

Diagnostics remain on stderr, so JSONL pipes are not contaminated. Network
disconnects and clean stream closes reconnect after one second. Reconnects
send the last completely received SSE event ID in `Last-Event-ID`; an
incomplete event is never emitted or used as a cursor. A server admission or
quota error exits non-zero instead of reconnecting indefinitely, and a valid
integer `Retry-After` is reported on stderr.

Press `Ctrl-C` to stop `events tail` or `events stream`. Both restore terminal
state as needed and exit successfully without an extra stdout record.

## Tokens

```bash
barestash tokens create \
  [--name NAME] \
  [--scope SCOPE ... | --preset read-only|full-access] \
  [--expires-in 30d | --no-expiration] \
  [--json]
barestash tokens list [--all] [--json]
barestash tokens revoke TOKEN_ID [--yes]
```

The available scopes are `endpoints:read`, `endpoints:write`, `events:read`,
`tokens:read`, `tokens:write`, and `mcp:use`. `read-only` grants endpoint/event
read plus MCP use; `full-access` grants all scopes. The server default lifetime
is 90 days when neither expiration option is given. Non-expiring creation prints
an explicit warning. A newly issued token secret is written only to stdout.

## Output and terminal behavior

- Human-readable output is the default.
- `--json` writes pretty JSON without ANSI escape sequences.
- `events stream` is always JSON Lines (NDJSON), one event per line.
- Results and machine data use stdout; progress, warnings, diagnostics, and
  errors use stderr.
- TTY output may use color, Unicode, and responsive tables.
- `NO_COLOR` disables ANSI color. `TERM=dumb` or non-TTY stdout selects plain
  output.
- Untrusted API and webhook text is made terminal-safe in human output: cursor,
  OSC/clipboard, and control sequences are displayed rather than executed.
  JSON and JSONL values are not altered by this terminal-only protection.

## Environment and local files

| Variable | Meaning |
| --- | --- |
| `BARESTASH_TOKEN` | Highest-priority bearer token for API requests |
| `BARESTASH_ENDPOINT` | Endpoint used when `--endpoint` is absent |
| `BARESTASH_API_URL` | API base URL; defaults to `http://localhost:8787` |
| `BARESTASH_ALLOW_INSECURE_API_URL` | `1` or `true` permits private/link-local API hosts |
| `BARESTASH_CONFIG_FILE` | Exact TOML config file override |
| `XDG_CONFIG_HOME` | Config root when no exact override is set |
| `NO_COLOR` | Disables ANSI color when present |

Default config paths are:

| Platform | Path |
| --- | --- |
| macOS | `~/Library/Application Support/barestash/config.toml` |
| Linux | `~/.config/barestash/config.toml` |
| Windows | `%APPDATA%\barestash\config.toml` |

The config format is TOML:

```toml
default_endpoint = "ep_abc123"
```

The legacy `token` key is still accepted as a final authentication fallback,
but new credentials should be stored with `barestash auth login`. A path
supplied through `BARESTASH_CONFIG_FILE` is always parsed as TOML, regardless
of its extension.

This config is separate from `credentials.json`, which is used only as the
explicitly warned plaintext credential fallback and remains JSON so its
credential and logged-out marker formats stay compatible.

API requests permit only HTTP(S) URLs without embedded credentials. Private,
link-local, and metadata destinations require the explicit insecure override.
Redirects are followed manually, revalidated at every hop, limited to the same
origin, and capped so an Authorization header cannot be redirected elsewhere.
Without the override, hostname DNS results are also checked for private or
link-local addresses and pinned for the process to prevent DNS rebinding.

## Development

The repository is a virtual Cargo workspace with four packages. Directory
names stay concise inside the repository while Cargo package names retain the
`barestash` identity:

- `crates/cli` (`barestash`): the executable and feature-oriented CLI
- `crates/client` (`barestash-client`): reusable HTTP and SSE transport
- `crates/local-state` (`barestash-local-state`): secure config, credentials,
  and locking
- `crates/protocol` (`barestash-protocol`): wire contracts and validation

The CLI groups clap inputs, orchestration, transformations, and presentation by
the `auth`, `endpoints`, `events`, and `tokens` features. Separate crates are
reserved for reusable transport/protocol code and the security-sensitive local
state boundary. All packages live under `crates/`; none are published to
crates.io. `just` is the documented command surface:

```bash
just install
just check
just check-package barestash-local-state
just test
just build
just package
just ci
```

`just check` covers every workspace package. `just package` explicitly verifies
the `barestash` release build and Cargo source-package allowlist; none of the
workspace packages are published to crates.io.

The Nix/direnv shell provides Rust, Cargo, rustfmt, Clippy, and repository tools.
See [docs/cli-design.md](docs/cli-design.md) for the observable contract and
[docs/rust-migration-inventory.md](docs/rust-migration-inventory.md) for the
reference-implementation inventory used during the rewrite.

## Security

Never include real tokens, endpoint secrets, private endpoint URLs, or captured
payloads in issues. Report vulnerabilities privately as described in
[SECURITY.md](SECURITY.md).
