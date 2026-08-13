# Native CLI release

The `barestash` CLI is distributed as native archives through GitHub Releases.
Node.js and npm are not part of the build, runtime, or publication path.

## Verification

Before tagging a release, update the version in `Cargo.toml` and run:

```bash
just ci
```

This checks formatting, runs Clippy with warnings denied, runs all tests,
verifies the Cargo package allowlist, and builds the release binary. The
GitHub Actions check workflow repeats the quality gates on Ubuntu, macOS, and
Windows.

## Publishing

Push an annotated `vMAJOR.MINOR.PATCH` tag only after the reviewed version has
landed on `main`. The release workflow builds these targets:

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
The Cargo package itself is not published to crates.io; its explicit file
allowlist exists to verify source packaging and excludes Node.js caches,
coverage output, and build artifacts.

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
