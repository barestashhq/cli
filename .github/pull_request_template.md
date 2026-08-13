## Context

<!-- What problem does this PR solve? Link related issues when relevant. -->

## Changes

<!-- Describe the main changes and any intentional behavior differences. -->

-

## CLI contracts and documentation

Check everything affected by this PR:

- [ ] Commands, arguments, flags, help text, errors, or exit codes
- [ ] Human-readable output, JSON/JSONL, or stdout/stderr behavior
- [ ] Endpoint or token resolution, local config, or credential storage
- [ ] API contracts or portable helpers in `src/protocol.rs`
- [ ] `README.md` or `docs/cli-design.md`
- [ ] Build, package, dependency, or release behavior
- [ ] No user-visible behavior or public contract changes

<!-- Explain any required coordination with the Barestash API separately. -->

## Security and data handling

- [ ] This PR contains no live tokens, refresh credentials, endpoint secrets,
      cookies, private endpoint URLs, captured payloads, or user data.
- [ ] Logs, errors, fixtures, and examples use synthetic or safely redacted data.
- [ ] Security-sensitive behavior is covered by tests, or is not affected.
- [ ] Destructive or remote side effects require explicit user intent, or are not
      affected.

## Verification

List the commands and manual checks that were run. Use `just check` for source or
configuration changes and `just package` for build, dependency, manifest, or
release changes. Use `just ci` before a release.

```text

```

Skipped checks or residual risk:

- None

## Review notes

<!-- Call out tradeoffs, compatibility concerns, or areas needing close review. -->
