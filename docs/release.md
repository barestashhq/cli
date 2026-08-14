# Native CLI release

The `barestash` CLI is distributed as native archives through GitHub Releases.
Node.js and npm are not part of the build, runtime, or publication path.

## Verification

Before tagging a release, update `version` under `[workspace.package]` in the
root `Cargo.toml` and run:

```bash
just ci
```

This checks formatting, runs Clippy with warnings denied, and runs tests across
all workspace packages. It also verifies the `barestash` Cargo package
allowlist, builds the release binary, and smoke-tests its version output. The
GitHub Actions CI workflow runs the same `just ci` gate on Ubuntu, macOS, and
Windows. Ubuntu runs first; macOS and Windows start only after it succeeds.

## Publishing

Push an annotated `vMAJOR.MINOR.PATCH` tag only after the reviewed version has
landed on `main`. The release workflow reuses the three-platform CI workflow,
validates that the tag matches the Cargo version, and then builds these targets:

| Target | Archive |
| --- | --- |
| `aarch64-apple-darwin` | `.tar.gz` |
| `x86_64-apple-darwin` | `.tar.gz` |
| `x86_64-unknown-linux-gnu` | `.tar.gz` |
| `aarch64-unknown-linux-gnu` | `.tar.gz` |
| `x86_64-pc-windows-msvc` | `.zip` |

Each archive contains the binary, README, license, and third-party notices. A
lowercase SHA-256 checksum is published beside each archive. Artifact names use
`barestash-vMAJOR.MINOR.PATCH-TARGET`, which can be consumed by a future
Homebrew tap without renaming.

Linux archives are built with `cross` for both architectures to avoid silently
raising the minimum glibc version when GitHub updates its hosted Ubuntu image.
None of the workspace packages are published to crates.io. The executable
`barestash` package keeps an explicit file allowlist to verify source packaging
and exclude Node.js caches, coverage output, and build artifacts. `just package`
also verifies that its packaged license and third-party notice remain identical
to the repository-root copies used by release archives.

## npm migration decision

The old `@barestash/cli` package and Node.js wrapper have been removed from this
repository. Reintroducing an npm compatibility shim would add another release
surface, platform package indirection, and a Node.js installation requirement
for users who discover the CLI through npm. Native GitHub Releases are the
authoritative path for this migration.

If ecosystem demand later justifies an npm shim, maintain it as a separately
reviewed compatibility project that downloads a checksummed native artifact;
do not embed or respawn the removed TypeScript implementation.

## Homebrew

No tap is published by this repository yet. The stable artifact names and
checksums are intentionally Homebrew-friendly. When a tap is added, its formula
should select the two macOS architectures, verify the release checksum, install
only the `barestash` binary, and smoke-test `barestash --version`.
