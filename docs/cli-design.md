# Barestash Command Design Specification

## Overview

Barestash CLI is designed around a resource/action command structure.

```text
barestash {resource} {action}
```

Examples:

```bash
barestash auth login
barestash endpoints create
barestash endpoints secrets create
barestash events tail
barestash events stream
barestash tokens create
```

The CLI should make the core Barestash loop simple and predictable:

```text
Receive webhooks
    ↓
Stash raw requests
    ↓
Stream events to CLI or AI agents
```

The command design should emphasize:

- Resource/action consistency
- CLI-first developer workflows
- Machine-readable output for scripts and AI agents
- Minimal dashboard dependency

## Native implementation boundaries

The shipped CLI is a native Rust binary. Its internal modules keep observable
concerns separate:

- `cli` owns the clap command tree and local argument validation.
- `protocol` owns typed JSON/SSE wire contracts.
- `domain` owns pure resolution, redaction, duration, credential, and body
  transformation rules.
- `error` owns shared application failure values without terminal rendering.
- `application` coordinates authentication and resource workflows without
  terminal rendering in domain logic.
- `infrastructure` owns reqwest/rustls HTTP, secure redirects, platform
  credential stores, atomic config files, file locks, SSE byte decoding, and
  terminal/browser adapters.
- `presentation` owns human, JSON, JSONL, and the intentionally simple tail
  view.

The binary never starts Node.js and no TypeScript runtime is part of release
artifacts. The migration inventory in `rust-migration-inventory.md` records the
reference behavior and the small set of deliberate validation/security fixes.

## Design Goals

### 2.1 Resource/action consistency

All primary commands should follow:

```text
barestash {resource} {action}
```

This keeps the CLI easy to learn and avoids a flat namespace with too many top-level verbs.

Preferred:

```bash
barestash events tail
barestash events stream
```

Avoid as primary commands:

```bash
barestash tail
barestash stream
```

Short aliases may be considered later, but the documented command surface should use the resource/action model.

### Human and machine output separation

Commands should clearly distinguish between:

- Human-readable terminal output: the default
- Machine-readable JSON output: explicit `--json`
- Machine-readable JSONL (NDJSON) output: `events stream`

Default output should be optimized for humans.

When stdout is an interactive terminal, human output uses ANSI color, Unicode
symbols, syntax highlighting, and tables fitted to the detected terminal
width. Narrow terminals truncate flexible columns and fall back to vertical
key/value rows when a table cannot fit. When stdout is redirected or piped, or
when `TERM=dumb`, human output remains undecorated plain text. The `NO_COLOR`
environment variable disables ANSI color while preserving the interactive
layout.

Human output treats endpoint, event, header, body, account, and API error text
as untrusted terminal input. Control sequences capable of cursor movement,
screen manipulation, OSC/clipboard access, or embedded line controls are
rendered visibly. Renderer-owned SGR color remains allowed. This protection is
not applied to JSON or JSONL serialization, whose data contract remains exact.

Machine-readable output must be explicitly requested. Structured JSON and
JSONL output must never contain ANSI escapes. For continuous machine-readable
streams, JSONL is preferred.

### CLI and AI agent friendliness

Barestash should support both interactive developer workflows and automation/agent workflows.

The CLI should be easy to use from:

- Terminals
- Shell scripts
- Local development processes
- CI jobs
- AI agents
- MCP-compatible tooling

## Command Resources

The resources are:

```text
auth
endpoints
  secrets (sub-resource of endpoints)
events
tokens
```

## Command Set

### Authentication Commands

This document defines the CLI-facing authentication, session, and token
contract. Server-side identity and storage internals are outside this
repository's scope.

#### `barestash auth login`

Authenticate the CLI with Barestash.

```bash
barestash auth login
```

Expected behavior:

- Starts the Barestash Device Authorization Flow
- Prints the verification URL and one-time user code, and opens the browser when possible
- Polls the device token endpoint using the server-provided interval
- Stores the CLI session credentials (access and refresh tokens) securely
- Replaces any existing stored credential (the CLI keeps at most one stored credential)
- Prints the authenticated account and session expiration

Example output:

```text
Open this URL in your browser:

  https://app.example.com/device

Enter this one-time code:

  JKLM-PQRS

Waiting for authorization...

✓ Authenticated as barestash@example.com
```

Flags:

- `--with-token` reads a Personal Access Token from stdin, validates it with `GET /v1/account`, and stores it without creating a refreshable CLI session
- `--insecure-storage` intentionally stores the resulting credential in a plaintext local file with restrictive permissions instead of using the operating system credential store

```bash
echo "$BARESTASH_TOKEN" | barestash auth login --with-token
barestash auth login --insecure-storage
```

By default, the CLI uses the operating system credential store. If that write
fails, it warns and falls back to the same restrictive plaintext storage used
by `--insecure-storage` (mode `0600` on Unix-like systems). Plaintext fallback
must never be silent.

#### `barestash auth logout`

Remove local authentication credentials.

```bash
barestash auth logout
```

Expected behavior:

- Deletes all locally stored credentials: interactive session access and refresh tokens, stored Personal Access Tokens, and local session metadata
- If either credential backend cannot be cleared, atomically replaces any remaining plaintext credential with a non-secret logout marker and treats the CLI as logged out without falling through to a potentially stale operating system credential; a later successful cleanup or login removes the marker
- Does not modify the `BARESTASH_TOKEN` environment variable
- Does not revoke remote credentials unless explicitly requested
- Use `barestash tokens revoke <token-id>` to revoke a specific token ID (`auth logout --revoke` targets the stored credential)

Flags:

- `--revoke`: revokes the remote credential matching the stored credential type before clearing local state
  - stored interactive CLI session: `POST /v1/auth/sessions/current/revoke`
  - stored Personal Access Token: `DELETE /v1/tokens/{token_id}` (self-revocation succeeds regardless of the PAT's scopes)
  - if the first revoke succeeds but the response is lost, a retry may receive `token_revoked`, `personal_access_token_expired`, `session_revoked`, or `session_expired`; during `--revoke` only, the CLI treats those as confirmed remote success and still clears local credentials

```bash
barestash auth logout --revoke
```

#### `barestash auth status`

Show current authentication status.

```bash
barestash auth status
```

Expected behavior:

- Calls `GET /v1/account` to resolve the current account and credential (does not rely on `GET /v1/tokens`)
- Displays the account, credential type, expiration, scopes where applicable, and default endpoint

Example output:

```text
Authenticated as barestash@example.com
Credential: cli_session (expires 2026-07-11T13:00:00Z)
Scopes: endpoints:read endpoints:write events:read tokens:read tokens:write mcp:use
Default endpoint: ep_abc123
```

Flags:

- `--json`: JSON output

```bash
barestash auth status --json
```

## Token Commands

Tokens are Personal Access Tokens: non-interactive authentication credentials for CI, scripts, and AI agents.

The `tokens` resource is the CLI surface for issuing, listing, and revoking Personal Access Tokens.

### `barestash tokens create`

Issue a Personal Access Token for CI, scripts, and AI agents.

```bash
barestash tokens create
```

Expected behavior:

- Requires an authenticated principal (interactive CLI session or a PAT with `tokens:write`)
- Requested scopes must be a subset of the authenticated principal's scopes; the CLI must not submit broader scopes and must surface backend `insufficient_scope` rejections clearly
- Displays the final resolved scopes before issuing the token in interactive mode
- Sends an `Idempotency-Key` header, generated per invocation, so internal retries cannot mint duplicate tokens
- Shows the token ID (e.g. `tok_abc123`) and secret once at creation time (not shown again)
- Does not show the secret in subsequent `tokens list` output
- Guides the user to use `BARESTASH_TOKEN` or `auth login --with-token` immediately after creation
- Default expiration is 90 days; `--no-expiration` requires explicit opt-in and prints a warning

Example output:

```text
Created token: tok_abc123

Token (shown once):
bst_pat_xxxxxxxx_xxxxxxxxxxxxxxxxxxxxxxxx

Save this token now. It will not be shown again.

Use it with:
  export BARESTASH_TOKEN=...
  echo "$BARESTASH_TOKEN" | barestash auth login --with-token
```

Flags:

- `--name <name>`: assign a human-readable name to the token (e.g. `ci-github`, `local-agent`)
- `--scope <scope>`: repeatable scope selection (e.g. `endpoints:read`, `events:read`)
- `--preset <read-only|full-access>`: `read-only` selects
  `endpoints:read`, `events:read`, and `mcp:use`; `full-access` selects
  every scope listed by the API
- `--expires-in <duration>`: expiration such as `30d`, `90d`, `1y` (converted to seconds for the API)
- `--no-expiration`: create a non-expiring token (maps to `"expires_in": null`)
- `--json`: JSON output (includes secret; for scripts)

```bash
barestash tokens create --name ci-github --scope endpoints:read --scope events:read --expires-in 90d
barestash tokens create --preset read-only
barestash tokens create --json
```

`tokens create` requires an authenticated CLI session or a scoped PAT with
`tokens:write`. It does not read or forward legacy bootstrap credentials.

### `barestash tokens list`

List metadata for issued tokens. Does not include secrets.

```bash
barestash tokens list
```

Expected behavior:

- Requires `tokens:read`
- Lists Personal Access Token metadata associated with the authenticated account
- Shows active tokens by default
- Shows revoked / expired tokens with `--all`
- Never includes raw token values

Example output:

```text
ID          NAME         SCOPES                       EXPIRES     LAST_USED   STATUS
tok_abc123  ci-github    endpoints:read,events:read   2026-10-09  2026-07-05  active
tok_def456  local-agent  full-access                  never       never       active
```

Flags:

- `--json`: JSON output
- `--all`: include revoked / expired tokens

```bash
barestash tokens list --json
barestash tokens list --all
```

### `barestash tokens revoke <token-id>`

Revoke a specified token. Used as part of token rotation.

```bash
barestash tokens revoke tok_abc123
```

Expected behavior:

- Requires `tokens:write`, except when revoking the PAT used to authenticate the request itself (self-revocation is exempt)
- Requires confirmation by default
- Revokes the token with the specified ID; revocation is idempotent
- Shows an additional warning when revoking the token currently used by the CLI
- `auth logout --revoke` targets the stored credential; `tokens revoke` is an admin command targeting any token ID and does not affect CLI sessions

Flags:

- `--yes`: revoke without prompting

```bash
barestash tokens revoke tok_abc123 --yes
```

## Endpoint Commands

Endpoints are receiving URLs for incoming webhooks or HTTP requests.

### `barestash endpoints create`

Create a new endpoint.

```bash
barestash endpoints create
```

Expected behavior:

- Creates an endpoint for receiving external requests
- Prints the endpoint ID and webhook URL
- With `--set-default`, stores the new endpoint as the local CLI default without
  changing the JSON response shape
- Without `--set-default`, leaves any existing default endpoint unchanged
- If endpoint creation succeeds but default persistence fails, still prints the
  created endpoint, reports the partial failure on stderr, and exits non-zero

Example output:

```text
Created endpoint: ep_abc123

Webhook URL:
https://ingest.{domain}/ep_abc123

Append a path suffix when the webhook provider requires it:
https://ingest.{domain}/ep_abc123/github/push
```

Flags:

- `--private`: create a private endpoint tied to the authenticated user (**default**)
- `--name <name>`: assign a human-readable name to the endpoint
- `--temporary`: create a temporary endpoint for unauthenticated or short-term use (see Temporary Endpoint Constraints below)
- `--set-default`: set the created endpoint as the CLI default endpoint

Private endpoint constraints:

| Attribute | Value |
| --- | --- |
| TTL | 7 days |
| Max stored events | 1000 (further ingest returns HTTP 429) |
| Creation auth | Required |
| Read access | Authentication required |
| Delete API | Supported before automatic expiry cleanup |
| Expiry cleanup | Automatic scheduled cleanup deletes the endpoint, events, secrets, and raw request objects |

Temporary endpoint constraints:

| Attribute | Value |
| --- | --- |
| TTL | 24 hours |
| Max stored events | 100 (further ingest returns HTTP 429) |
| Creation auth | Not required |
| Read access | Public-by-URL (no authentication required) |
| Use case | Non-sensitive, short-term debugging only |

```bash
barestash endpoints create --private
barestash endpoints create --name github-dev
barestash endpoints create --temporary
barestash endpoints create --set-default
```

### `barestash endpoints list`

List available endpoints.

```bash
barestash endpoints list
```

Expected behavior:

- Requires authentication.
- Lists active, unexpired private endpoints for the authenticated account.
- Does not anonymously enumerate temporary endpoint IDs.
- Temporary endpoint details remain available through `endpoints show <endpoint-id>` when the endpoint ID is known.

Example output:

```text
ID          NAME          MODE        CREATED
ep_abc123   github-dev    private     2026-07-05
ep_def456   stripe-test   temporary   2026-07-05
```

Flags:

- `--json`: JSON output

```bash
barestash endpoints list --json
```

### `barestash endpoints show <endpoint-id>`

Show endpoint details.

```bash
barestash endpoints show ep_abc123
```

Example output (private endpoint):

```text
Endpoint: ep_abc123
Name: github-dev
Webhook URL: https://ingest.{domain}/ep_abc123
Path suffix (optional): https://ingest.{domain}/ep_abc123/github/push
Mode: private
Expires: 2026-07-12T12:00:00Z
Events: 0 / 1000
Created: 2026-07-05T12:00:00+09:00
```

Example output (temporary endpoint):

```text
Endpoint: ep_def456
Name: stripe-test
Webhook URL: https://ingest.{domain}/ep_def456
Mode: temporary
Expires: 2026-07-06T12:00:00Z
Events: 42 / 100
Public read: yes (no authentication required)
Created: 2026-07-05T12:00:00Z
```

Flags:

- `--json`: JSON output

```bash
barestash endpoints show ep_abc123 --json
```

### Endpoint Secret Commands

Private endpoints support optional ingest secret verification via the `x-barestash-secret` header. The CLI exposes secret management under the `endpoints secrets` sub-resource.

Secret commands apply to **private endpoints only**. Temporary endpoints do not support ingest secrets.

The raw secret value and `x-barestash-secret` must never appear in event output or CLI logs.

#### `barestash endpoints secrets create`

Create a new ingest secret for a private endpoint.

```bash
barestash endpoints secrets create
```

Expected behavior:

- Requires authentication
- Creates a new active secret via `POST /v1/endpoints/{endpoint_id}/secrets`
- Shows the secret ID (e.g. `sec_abc123`) and raw secret **once** at creation time
- Guides the user to configure the external webhook provider with `x-barestash-secret`
- Supports multiple active secrets during rotation

Example output:

```text
Created secret: sec_abc123

Secret (shown once):
xxxxxxxxxxxxxxxxxxxxxxxx

Save this secret now. It will not be shown again.

Configure your webhook provider to send:
  x-barestash-secret: xxxxxxxxxxxxxxxxxxxxxxxx
```

Flags:

- `--endpoint <endpoint-id>`: target endpoint (uses default endpoint resolution when omitted)
- `--json`: JSON output (includes secret; for scripts)

```bash
barestash endpoints secrets create --endpoint ep_abc123
barestash endpoints secrets create --json
```

#### `barestash endpoints secrets list`

List ingest secret metadata for a private endpoint. Does not include raw secret values.

```bash
barestash endpoints secrets list
```

Expected behavior:

- Requires authentication
- Lists secrets via `GET /v1/endpoints/{endpoint_id}/secrets`
- Shows secret ID, status, created time, and last used time

Example output:

```text
ID          STATUS   CREATED     LAST_USED
sec_abc123  active   2026-07-05  2026-07-05
sec_def456  revoked  2026-07-01  2026-07-04
```

Flags:

- `--endpoint <endpoint-id>`: target endpoint
- `--json`: JSON output

```bash
barestash endpoints secrets list --endpoint ep_abc123
barestash endpoints secrets list --json
```

#### `barestash endpoints secrets revoke <secret-id>`

Revoke an ingest secret. Used as part of secret rotation.

```bash
barestash endpoints secrets revoke sec_abc123
```

Expected behavior:

- Requires authentication
- Revokes the secret via `DELETE /v1/endpoints/{endpoint_id}/secrets/{secret_id}`
- Requires confirmation by default

Flags:

- `--endpoint <endpoint-id>`: target endpoint (required when the secret belongs to a non-default endpoint)
- `--yes`: revoke without prompting

```bash
barestash endpoints secrets revoke sec_abc123 --yes
```

### `barestash endpoints delete <endpoint-id>`

Delete an endpoint.

```bash
barestash endpoints delete ep_abc123
```

Expected behavior:

- Requires confirmation by default
- Deletes **private endpoints only** before automatic expiry cleanup
- Deletes the endpoint and all associated events and stored request bodies
- Rejects deletion of temporary endpoints with a clear error (temporary endpoints expire automatically after 24 hours; deletion is not supported in MVP)

Flags:

- `--yes`: delete without prompting

```bash
barestash endpoints delete ep_abc123 --yes
```

## Event Commands

Events represent incoming requests captured by Barestash.

The `events` resource should be the core CLI surface for reading, following, and streaming captured webhooks.

Manual event deletion or purge is not supported. Events may be removed only by deleting the parent endpoint or by scheduled retention cleanup.

### `barestash events list`

List received events.

```bash
barestash events list
```

Example output:

```text
ID              METHOD  PATH              CONTENT-TYPE       SIZE    RECEIVED
evt_01JABC      POST    /webhook/github    application/json   2.1KB   12:04:18
evt_01JDEF      POST    /webhook/stripe    application/json   8.4KB   12:04:32
```

Expected behavior:

- Lists recent events for the default endpoint
- Supports endpoint selection
- Supports JSON output for scripts

Flags:

- `--endpoint`: specify an endpoint
- `--limit`: specify the number of events to fetch
- `--json`: JSON output

```bash
barestash events list --endpoint ep_abc123
barestash events list --limit 20
barestash events list --json
```

### `barestash events latest`

Show the most recently received event.

```bash
barestash events latest
```

Expected behavior:

- Fetches the latest event for the default endpoint
- Pretty-prints JSON body when possible
- Displays sensitive headers as `[REDACTED]` (aligned with backend API behavior)
- Does not redact body content
- Does not provide a flag to show raw sensitive header values in MVP
- With `--json`, emits `{ "event": null, "body": null }` when no events exist

Example output:

```text
Event: evt_01JDEF

Request:
  Method:       POST
  Path:         /webhook/stripe
  Received:     2026-07-05T12:04:32+09:00
  Content-Type: application/json
  Size:         8.4KB

Headers:
  content-type: application/json
  stripe-signature: [REDACTED]
  user-agent: Stripe/1.0 (+https://stripe.com/docs/webhooks)

Body:
{
  "id": "evt_...",
  "type": "checkout.session.completed",
  "data": {
    "object": {
      "id": "cs_..."
    }
  }
}
```

Flags:

- `--endpoint`: specify an endpoint
- `--json`: JSON output

```bash
barestash events latest --endpoint ep_abc123
barestash events latest --json
```

### `barestash events show <event-id>`

Show details for a captured event.

```bash
barestash events show evt_01JDEF
```

Expected behavior:

- Fetches the specified event
- Shows a detail view including request metadata, headers, and body by default
- Pretty-prints JSON body when possible
- Displays sensitive headers as `[REDACTED]` (aligned with backend API behavior)
- Does not redact body content
- Does not provide a flag to show raw sensitive header values in MVP

Example output:

```text
Event: evt_01JDEF
Endpoint: ep_abc123

Request:
  Method:       POST
  Path:         /webhook/stripe
  Received:     2026-07-05T12:04:32+09:00
  Content-Type: application/json
  Size:         8.4KB

Headers:
  content-type: application/json
  stripe-signature: [REDACTED]
  user-agent: Stripe/1.0 (+https://stripe.com/docs/webhooks)

Body:
{
  "id": "evt_...",
  "type": "checkout.session.completed",
  "data": {
    "object": {
      "id": "cs_..."
    }
  }
}
```

Flags:

- `--json`: JSON output

```bash
barestash events show evt_01JDEF --json
```

### `barestash events tail`

Follow incoming events in a human-readable terminal view.

This is the webhook equivalent of `tail -f`.

```bash
barestash events tail
```

Expected behavior:

- Watches the default endpoint for new events via polling
- Prints a column header before event rows
- Starts the watch display only after the initial event-list request succeeds,
  so the API host diagnostic appears before the human-readable layout
- Prints each new event as it arrives
- Optimizes output for human readability
- Exits successfully without additional output when interrupted with `Ctrl+C`
- When `--headers` is set, displays sensitive headers as `[REDACTED]`
- When `--body` is set, pretty-prints JSON body when possible; displays binary/multipart bodies as content type and size only
- Does not provide a flag to show raw sensitive header values in MVP

Example output:

```text
Watching endpoint: ep_abc123

RECEIVED                   METHOD PATH            SIZE CONTENT-TYPE     EVENT
[12:04:18] POST /webhook/github 2.1KB application/json evt_01JABC
[12:04:32] POST /webhook/stripe 8.4KB application/json evt_01JDEF
```

Flags:

- `--endpoint`: specify an endpoint
- `--last`: show the last N events before watching begins
- `--headers`: include headers in output
- `--body`: include body in output
- `--view`: show a screen-updating dashboard in an interactive terminal
- `--poll-interval`: specify polling interval (default `2s`; supports `ms` / `s` / `m`; unitless values are not allowed)

```bash
barestash events tail --endpoint ep_abc123
barestash events tail --last 10
barestash events tail --headers
barestash events tail --body
barestash events tail --view
barestash events tail --poll-interval 2s
```

`--view` displays the selected endpoint, watch status, number of events
received by the dashboard, latest event time, and up to ten recent event rows.
Below the full table width it uses a compact table without size and content
type columns. It limits visible events further when needed so the rendered
frame fits the detected terminal height, and reads current terminal dimensions
again on every redraw so resizing takes effect. It redraws only stdout and
exits silently and successfully on `Ctrl+C`, like the regular tail display.
On exit it resets terminal styling and moves the cursor to a fresh line. The
option requires interactive stdout with terminal control support, rejects
`TERM=dumb`, and is incompatible with `--headers` and `--body`; invalid
combinations fail before making an API request. API host diagnostics and errors
continue to use stderr.

Example dashboard:

```text
BARESTASH  ep_abc123

  Status      ● watching
  Requests    2
  Last event  2026-07-05T12:04:32.000Z

TIME      METHOD  PATH                 SIZE    CONTENT-TYPE      EVENT
12:04:32  POST    /webhook/stripe      8.4 KB  application/json  evt_01JDEF
12:04:18  POST    /webhook/github      2.1 KB  application/json  evt_01JABC
```

Example output for `barestash events tail --headers --body`:

```text
Watching endpoint: ep_abc123

RECEIVED                   METHOD PATH            SIZE CONTENT-TYPE     EVENT
[12:04:32] POST /webhook/stripe 8.4KB application/json evt_01JDEF

Headers:
  content-type: application/json
  user-agent: Stripe/1.0 (+https://stripe.com/docs/webhooks)
  stripe-signature: [REDACTED]

Body:
{
  "id": "evt_...",
  "type": "checkout.session.completed",
  "data": {
    "object": {
      "id": "cs_..."
    }
  }
}
```

Implementation sketch:

```text
barestash events tail
    ↓
GET /v1/endpoints/{endpoint_id}/events?after={cursor}
    ↓
Print new events
    ↓
Repeat
```

### `barestash events stream`

Stream incoming events in a machine-readable format for scripts, automation, and AI agents.

`barestash events stream` outputs JSON Lines / NDJSON only.
Each line represents one transformed event object.

```bash
barestash events stream
```

Expected behavior:

- Supports authenticated private endpoints only
- Subscribes to the backend SSE stream via `GET /v1/endpoints/{endpoint_id}/events/stream`
- Transforms each SSE `data` payload into one JSONL line on stdout
- Outputs JSON Lines / NDJSON for machine consumers
- Each line includes event metadata, request metadata, and a decoded body
- Can be piped into other tools, scripts, automation, or AI agent runtimes
- Does not produce human-readable display output
- Exits successfully without emitting a partial JSONL record or diagnostic when interrupted with `Ctrl+C`
- Displays sensitive headers as `[REDACTED]` in JSONL output
- Supports reconnect with `Last-Event-ID` (handled by the CLI SSE client)
- For a temporary endpoint, prints the backend policy error once to stderr,
  writes no JSONL record, and exits without reconnecting
- For an initial concurrency or daily-quota rejection, prints the backend
  error and actionable guidance to stderr, writes no JSONL record, and exits
  non-zero without reconnecting
- Displays the daily quota's UTC reset guidance and numeric `Retry-After`
  delay in seconds
- If a clean stream EOF is followed by a daily-quota rejection on reconnect,
  exits non-zero without making another reconnect attempt

Implementation sketch:

```text
barestash events stream
    ↓
GET /v1/endpoints/{endpoint_id}/events/stream  (SSE)
    ↓
For each SSE message:
  decode base64 body
  transform payload
  write one JSONL line to stdout
```

JSONL transformation rules:

| Field | Rule |
| --- | --- |
| Base structure | Derived from SSE `data` JSON |
| `received_at` | UTC ISO 8601 (e.g. `2026-07-05T12:04:32.000Z`) |
| `request.headers` | Sensitive headers shown as `[REDACTED]` |
| `request.path` | Preserved from SSE `request.path` |
| `request.body_size` | Preserved from SSE `request.body_size` |
| `request.body_sha256` | Preserved from SSE `request.body_sha256` |
| `body` | Top-level field. SSE base64 body is decoded and transformed by content type |
| JSON body | Parsed JSON object |
| Text body | UTF-8 string (invalid UTF-8 falls back to base64 string) |
| Binary / multipart / empty | `{ "content_type": "...", "size": N }` only |

Example output:

```jsonl
{"id":"evt_01JABC","endpoint_id":"ep_abc123","received_at":"2026-07-05T12:04:18.000Z","request":{"method":"POST","path":"/webhook/github","query":{},"headers":{"content-type":"application/json","x-github-event":"push"},"body_size":2150,"body_sha256":"..."},"body":{"ref":"refs/heads/main","repository":{"name":"barestash"}}}
{"id":"evt_01JDEF","endpoint_id":"ep_abc123","received_at":"2026-07-05T12:04:32.000Z","request":{"method":"POST","path":"/webhook/stripe","query":{},"headers":{"content-type":"application/json","stripe-signature":"[REDACTED]"},"body_size":8400,"body_sha256":"..."},"body":{"id":"evt_...","type":"checkout.session.completed"}}
```

Flags:

- `--endpoint <endpoint-id>`: specify an endpoint

Future option (not required for MVP):

- `--metadata-only`: stream metadata without body payloads

```bash
barestash events stream --endpoint ep_abc123
```

## 9. Endpoint Selection

Most event commands should operate on a default endpoint.

Default endpoint can be set by:

- `barestash endpoints create --set-default`
- Environment variable
- Command flag

Recommended precedence:

1. `--endpoint <endpoint-id>` flag
2. `BARESTASH_ENDPOINT` environment variable
3. Local CLI config default endpoint
4. Error with a helpful message

The local default applies to event commands and endpoint-secret commands that
accept `--endpoint`. Expired or deleted defaults remain configured until
explicitly replaced; commands surface the corresponding endpoint API error.

Example error:

```text
No endpoint selected.

Run:
  barestash endpoints create --set-default

Or specify one:
  barestash events tail --endpoint ep_abc123
```

## Authentication and Configuration

### Authentication requirements by endpoint mode

| Operation | Private endpoint | Temporary endpoint |
| --- | --- | --- |
| `endpoints create` | Authentication required | Authentication not required |
| `endpoints secrets` commands | Authentication required | Not supported |
| `events list` / `latest` / `show` / `tail` | Authentication required | No authentication required when the endpoint ID is selected |
| `events stream` | Authentication required | Not supported; use `events tail` |
| `endpoints delete` | Authentication required; deletion supported before automatic expiry cleanup | Deletion not supported in MVP |

When operating on a temporary endpoint selected by `--endpoint`,
`BARESTASH_ENDPOINT`, or the local default, the CLI must not require
authentication for supported REST and polling operations. `events stream` is
private-endpoint-only.

### Token discovery

Recommended precedence:

1. `--with-token` flag
2. `BARESTASH_TOKEN` environment variable
3. The stored credential (interactive CLI session or stored Personal Access Token; the CLI keeps at most one)

Interactive sessions are created by `barestash auth login` (Device Authorization Flow). Personal Access Tokens are issued via `barestash tokens create`. A PAT supplied through `BARESTASH_TOKEN` is used directly and is never converted into a refreshable CLI session.

`barestash tokens create` follows the same discovery order and fails with an
actionable authentication error when no credential is available. Legacy
bootstrap environment variables are ignored.

### API base URL

The CLI resolves the Barestash API from `BARESTASH_API_URL` (default:
`http://localhost:8787`).

Security expectations:

- `BARESTASH_API_URL` must use `http:` or `https:` and must not embed credentials
  in the URL.
- Private, link-local, and cloud-metadata addresses are rejected by default so
  stored or environment-provided API tokens are not sent to unsafe hosts.
- Validation is deferred until the first API request. Help and version commands
  do not require a valid `BARESTASH_API_URL`.
- HTTP redirects are handled manually with a capped hop count. Each redirect
  target is re-validated with the same rules as the initial URL. Cross-origin
  redirects are rejected so credentials are not forwarded to another host.
- On first real API use, the CLI logs the resolved API host to stderr before
  API-backed result output or monitoring layouts are printed.
- Use `--allow-insecure-api-url` or `BARESTASH_ALLOW_INSECURE_API_URL=1` only
  when you intentionally target a private-network API host, such as a LAN
  development deployment.

Threat model:

- A malicious or mistyped `BARESTASH_API_URL` can cause the CLI to send Bearer
  tokens to an attacker-controlled host.
- Default redirect following can amplify SSRF-style probing from the machine
  running the CLI.
- Shared CI runners and multi-tenant hosts are especially sensitive because
  environment variables may be influenced by untrusted parties.

### Local config

The CLI stores local config as TOML in an OS-appropriate config directory.

Examples:

```text
macOS: ~/Library/Application Support/barestash/config.toml
Linux: ~/.config/barestash/config.toml
Windows: %APPDATA%\barestash\config.toml
```

The supported keys are:

```toml
default_endpoint = "ep_abc123"

# Legacy final authentication fallback only. New credentials belong in the
# operating system credential store or the explicit plaintext fallback.
# token = "bst_pat_..."
```

`BARESTASH_CONFIG_FILE` replaces the default path exactly. Its contents are
always parsed as TOML, even when the filename has another extension.

The TOML config must not be confused with `credentials.json` in the same
directory. `credentials.json` remains the user-only plaintext fallback for
credential records and the logged-out marker; it is authoritative over an old
keyring entry when present. Config and credential mutations share the
`credentials.lock` advisory lock.

### Non-interactive environments

CI, scripts, and AI agents should be able to authenticate without a browser.

Recommended approach: create a scoped Personal Access Token from an interactive session, then supply it to the non-interactive environment.

```bash
# On a machine with an interactive session
barestash auth login
barestash tokens create --name ci-github --scope endpoints:read --scope events:read

# In CI, scripts, or agents
export BARESTASH_TOKEN=bst_pat_...
barestash events stream
```

Or use an existing token directly:

```bash
BARESTASH_TOKEN=... barestash events stream
```

Or store it for repeated CLI use:

```bash
echo "$BARESTASH_TOKEN" | barestash auth login --with-token
cat token.txt | barestash auth login --with-token
barestash auth login --with-token < token.txt
```

## Error Handling Principles

Errors should be actionable.

### Monitoring command interruption

`barestash events tail` and `barestash events stream` are long-running
monitoring commands. A `SIGINT`, normally sent by `Ctrl+C`, is an expected user
action and must stop either command with exit code `0` without additional
stdout or stderr output. This behavior applies in both TTY and non-TTY
environments so pipelines receive the same exit status.

Other commands and signals retain their normal process semantics. In
particular, `SIGTERM` is not converted to a successful exit, and a second
`SIGINT` during shutdown may use the runtime's default forced-interruption
behavior.

### Missing authentication

```text
Not authenticated.

Run:
  barestash auth login

For non-interactive environments, set:
  BARESTASH_TOKEN

Note: authentication is not required for supported reads when using a
temporary endpoint with --endpoint (public-by-URL read access). Live streaming
is not supported; use `barestash events tail` instead.
```

### Cannot stream a temporary endpoint

```text
Live streaming is not supported for temporary endpoints.

Use polling instead:
  barestash events tail --endpoint ep_abc123

Or create and set a private endpoint:
  barestash endpoints create --private --set-default
```

The CLI treats `temporary_endpoint_stream_not_supported` as non-retryable. It
prints the error once to stderr, writes nothing to stdout, and exits with code
`1` without sleeping or reconnecting.

### Live stream concurrency limit

```text
Live stream subscriber limit reached for this account (5).

Close another live stream before retrying.
```

The CLI treats `stream_concurrency_limit_exceeded` as non-retryable. It prints
the backend message and guidance to stderr, writes nothing to stdout, and exits
with code `1` without sleeping or reconnecting.

### Live stream daily quota

```text
Daily live stream quota reached. Try again after 2026-08-08T00:00:00.000Z.

Retry-After: 10800 seconds.
```

The backend message supplies the UTC reset time, and the CLI displays a valid
numeric `Retry-After` header as seconds. An initial
`stream_daily_quota_exceeded` response is non-retryable and leaves stdout
empty. If an established stream ends cleanly because the quota is exhausted,
the existing reconnect attempt is allowed; when it receives the 429 response,
the CLI prints the error once and exits with code `1` without reconnecting
again.

### Cannot delete temporary endpoint

```text
Cannot delete temporary endpoint: ep_def456

Temporary endpoints expire automatically after 24 hours.
Deletion is not supported in MVP.

Create a new temporary endpoint if needed:
  barestash endpoints create --temporary
```

### Missing endpoint

```text
No endpoint selected.

Run:
  barestash endpoints create --set-default

Or specify:
  --endpoint ep_abc123
```

### No events received

```text
No events received yet.

Send a webhook to:
  https://ingest.{domain}/ep_abc123

Or with a path suffix:
  https://ingest.{domain}/ep_abc123/github/push
```

### Invalid event ID

```text
Event not found: evt_01JDEF

Run:
  barestash events list
```

### Invalid token ID

```text
Token not found: tok_abc123

Run:
  barestash tokens list
```

### API Error Mapping

CLI error messages should map to backend `error.code` values from the REST API.

For a stored interactive session, the CLI refreshes an expired or
near-expiry access token once and retries the original request once. It does
not refresh Personal Access Tokens. Terminal refresh failures
(`refresh_token_expired`, `refresh_token_revoked`,
`refresh_token_reuse_detected`, `session_expired`, `session_revoked`, or
`account_disabled`) clear the stored credential when possible and direct the
user to authenticate again.

| Backend `error.code` | Typical HTTP | CLI message guidance |
| --- | --- | --- |
| `invalid_request` | 400 | Request was invalid. Show the backend message and suggest checking command arguments or JSON input. |
| `endpoint_not_found` | 404 | Endpoint not found. Suggest `barestash endpoints create --set-default` or `barestash endpoints list`. |
| `endpoint_expired` | 410 | Endpoint expired. Suggest `barestash endpoints create --set-default`. |
| `not_authenticated` | 401 | Use the Missing authentication message above. |
| `not_authorized` | 403 | Not authorized to access this endpoint or resource. |
| `insufficient_scope` | 403 | The credential lacks a required scope. Show the missing scope and suggest creating a token with broader scopes from an interactive session. |
| `personal_access_token_expired` | 401 | The Personal Access Token has expired. Suggest `barestash tokens create` from an interactive session. Do not attempt refresh. |
| `temporary_endpoint_delete_not_supported` | 400 | Cannot delete temporary endpoint. Explain that temporary endpoints expire automatically and suggest creating a new temporary endpoint if needed. |
| `temporary_endpoint_stream_not_supported` | 400 | Live streaming is unavailable for temporary endpoints. Suggest `events tail` or creating a private endpoint; do not reconnect. |
| `event_limit_exceeded` | 429 | Endpoint has reached its configured event limit. Suggest creating and setting a new default endpoint. |
| `stream_concurrency_limit_exceeded` | 429 | Show the backend message and advise closing another live stream before retrying. Keep stdout empty and do not reconnect. |
| `stream_daily_quota_exceeded` | 429 | Show the backend UTC reset guidance and numeric `Retry-After` delay. Keep stdout empty and stop reconnecting. |
| `event_not_found` | 404 | Use the Invalid event ID message above. |
| `body_not_found` | 404 | Body unavailable for event `{event_id}`. |
| `internal_error` | 500 | An unexpected error occurred. Retry later. |

Example messages:

```text
Endpoint not found: ep_abc123

Run:
  barestash endpoints create --set-default
```

```text
Endpoint expired: ep_def456

Run:
  barestash endpoints create --set-default
```

```text
Endpoint event limit reached: ep_def456 (100/100 or 1000/1000)

Run:
  barestash endpoints create --set-default
```

```text
Not authorized to access endpoint: ep_abc123
```

## Naming Guidelines

### Use plural resource names

Preferred:

```bash
barestash endpoints list
barestash events list
```

Avoid:

```bash
barestash endpoint list
barestash event list
```

### Use standard action names

Preferred actions:

```text
login
logout
status
create
list
show
delete
revoke
latest
tail
stream
```

Sub-resource actions under `endpoints`:

```text
barestash endpoints secrets create
barestash endpoints secrets list
barestash endpoints secrets revoke
```

## Final Positioning in CLI Terms

These commands should express the core product promise:

```text
Receive webhooks, stash requests, and stream events to your CLI or AI agents.
```
