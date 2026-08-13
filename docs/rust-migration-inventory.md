# Rust migration behavior inventory

This inventory records the externally observable contract extracted from the
TypeScript implementation, its tests, and `docs/cli-design.md` before the Rust
implementation began. The old implementation is the reference for unspecified
details; security fixes are called out instead of silently preserving unsafe
behavior.

## Command and process contract

- Commands use `barestash {resource} {action}` with resources `auth`,
  `endpoints`, `events`, and `tokens`.
- The root accepts `--help` / `-h`, `--version` / `-V`, and the global
  `--allow-insecure-api-url` flag at any argument position.
- Human results go to stdout. Warnings, progress, API host diagnostics, and
  errors go to stderr. Expected local and API errors do not include stack
  traces. Success exits `0`; command, API, and validation failures exit `1`.
- JSON is explicit through `--json`; it is pretty-printed and never decorated.
  `events stream` always writes one compact JSON value per stdout line. It
  never writes human diagnostics or ANSI sequences to stdout.
- Only `events tail` and `events stream` turn `SIGINT` into silent successful
  cancellation. `events tail --view` restores terminal styling and moves to a
  fresh line before returning.

## Resolution and local state

- Endpoint precedence is `--endpoint`, non-undefined `BARESTASH_ENDPOINT`,
  then `default_endpoint` in config. An absent selection is an actionable
  local error.
- Authentication precedence is a non-empty `BARESTASH_TOKEN`, the stored
  credential, then the legacy config `token`. Public-by-ID REST reads suppress
  session refresh API/network failures and retry without authentication;
  authenticated commands and SSE do not.
- Config path precedence is `BARESTASH_CONFIG_FILE`, non-empty
  `XDG_CONFIG_HOME`, then the platform path:
  `~/Library/Application Support/barestash/config.toml` on macOS,
  `%APPDATA%\barestash\config.toml` on Windows, and
  `~/.config/barestash/config.toml` elsewhere. An exact override is parsed as
  TOML regardless of its extension. This is an intentional format change.
- TOML config contains `default_endpoint` and, as a final authentication
  fallback only, legacy `token`. User-only permissions are required.
- The system credential identifier is service `barestash`, account `default`.
  Stored credential JSON is either a Personal Access Token or a refreshable
  CLI session. `credentials.json` beside the TOML config remains the plaintext
  fallback, and
  `credentials.lock` serializes auth and config mutations.
- A valid plaintext credential or `{ "version": 1, "state": "logged_out" }`
  is authoritative over the keyring. Plaintext writes are atomic and mode
  `0600` on Unix; Windows applies a current-user-only ACL. Keyring fallback is
  always reported with the path.

## HTTP and security

- The API base is `BARESTASH_API_URL` or `http://localhost:8787`. Validation is
  lazy so help/version work even with an invalid environment value.
- Only HTTP(S) without embedded credentials is accepted. Private, link-local,
  and metadata destinations are rejected unless insecure access is explicitly
  enabled; loopback localhost remains available for development.
- Redirect following is manual, same-origin only, revalidates every target,
  rejects a missing `Location`, and permits at most five hops. `307`/`308`
  preserve the request; `303` converts non-GET/HEAD to GET; `301`/`302`
  convert POST to GET and otherwise preserve the method.
- Sensitive headers are normalized to lowercase. Authorization, cookies,
  common API/signature headers, and provider signatures become `[REDACTED]`.
  `x-barestash-secret` and `x-barestash-bootstrap-token` are removed entirely.

## Authentication

- Device login creates an authorization with all six scopes, displays the
  verification URI and code on stderr, opens the complete URI, sleeps before
  each poll, keeps the supplied interval for `authorization_pending`, and adds
  five seconds for each `slow_down` response.
- A successful device token must pass a refresh-free `GET /v1/account` and be
  a CLI access credential. Failure to validate or persist it triggers
  best-effort remote session revocation and a warning if cleanup is uncertain.
- `auth login --with-token` reads and trims stdin, validates with
  `GET /v1/account`, and accepts only a PAT.
- CLI access tokens refresh within a five-minute expiry window. Refresh rotates
  both tokens under the credential lock. An `access_token_expired` API response
  refreshes and retries the original request exactly once.
- Terminal refresh failures clear stale local credentials when possible.
  Persistence failure after rotation revokes the rotated session, clears stale
  state, preserves the original error, and warns about cleanup failures.
- `auth logout --revoke` targets the stored credential, not
  `BARESTASH_TOKEN`. It distinguishes sessions and PATs, treats already
  revoked/expired remote state idempotently, and does not erase a credential
  concurrently replaced by another login.

## API resources

- Endpoint APIs use `/v1/endpoints`, `/v1/endpoints/{id}`, and the nested
  `/secrets` resource. Private is the create default; `--temporary` is
  unauthenticated. `--private` and `--temporary` conflict. A `--set-default`
  persistence failure still prints the created endpoint and exits non-zero.
- Token APIs use `/v1/tokens`; creation validates requested scopes against
  `/v1/account` when a credential is present and sends a per-invocation
  `Idempotency-Key`. The scope set is `endpoints:read`, `endpoints:write`,
  `events:read`, `tokens:read`, `tokens:write`, and `mcp:use`. Read-only is
  `endpoints:read events:read mcp:use`; default/full-access is all scopes.
- Event REST APIs list under `/v1/endpoints/{id}/events`, retrieve detail at
  `/v1/events/{id}`, and fetch raw bodies from `/v1/events/{id}/body`.
- JSON content is decoded as UTF-8 and parsed; malformed JSON remains text.
  Text is UTF-8, while empty, multipart, binary, and invalid direct text bodies
  become `{content_type,size}` metadata. Streaming invalid UTF-8 JSON/text
  falls back to the original base64. Body content itself is not redacted.

## Tail, SSE, and terminal behavior

- Polling tail defaults to `2s`. `--last` is a non-negative integer. With
  `--last 0`, an initial `limit=1` establishes the cursor without printing the
  existing event. With `--last N`, initial newest-first events are reversed for
  chronological output before polling with `after`.
- SSE framing accepts LF or CRLF blank-line boundaries, supports multi-line
  `data:`, and buffers across byte and UTF-8 chunk boundaries. Only complete
  messages update output and Last-Event-ID.
- Network errors, stream read failures, incomplete messages, and clean EOF
  reconnect after one second. The next request sends `Last-Event-ID` from the
  last complete event. Invalid payload JSON is fatal and is not reconnected.
- A network failure while resolving or reactively refreshing the stream's CLI
  session follows the same reconnect path; refresh API rejection or local
  credential-persistence failure remains fatal.
- HTTP admission/API errors are not reconnected. Daily quota errors display a
  valid non-negative integer `Retry-After` in seconds; JSONL stdout remains
  empty for an initial rejection.
- Interactive mode requires stdout TTY and `TERM != dumb`. `NO_COLOR` disables
  color without disabling Unicode/responsive layout. The simple `--view`
  dashboard uses clear/home redraws, current width and height, at most ten
  newest events, a compact layout below 66 columns, and no raw mode or alternate
  screen.

## Deliberate Rust-side decisions

- Config writes are atomic and Windows config permissions match credential
  safety. These close durability/security gaps in the reference implementation.
- Public-looking API hostnames are resolved once, checked against the same
  private/link-local policy, and pinned to the validated addresses. This closes
  the reference implementation's DNS-rebinding gap; localhost and the explicit
  insecure override retain their documented behavior.
- The PID/staleness lock-file protocol is replaced with a persistent file
  protected by the operating system's advisory lock. Process exit releases the
  lock without stale-PID deletion races; the 10-second acquisition timeout and
  shared credential/config critical section remain.
- On Linux, the credential adapter checks both the current Secret Service
  `service`/`username` entry and keytar's legacy `service`/`account` entry so
  existing credentials migrate in place.
- On Windows, the adapter also checks keytar's legacy `service/account`
  Credential Manager target before using the current backend target. macOS
  continues to use the compatible Keychain service/account pair directly.
- Human output now neutralizes webhook/API terminal control sequences while
  leaving JSON and JSONL values unchanged. This intentionally fixes a terminal
  injection flaw instead of reproducing it for byte-for-byte compatibility.
- The documented current-token warning for `tokens revoke` is implemented even
  though the TypeScript presentation layer omitted it.
