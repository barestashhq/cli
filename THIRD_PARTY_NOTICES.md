# Third-party notices

The native Barestash CLI is built from Rust crates resolved in `Cargo.lock`.
The dependency graph was re-audited during the Rust migration; Commander.js,
the JavaScript string-width packages, esbuild, Vitest, and `@github/keytar` are
not linked into or required by the native binary.

## Direct runtime crates

| Crate | Purpose | License |
| --- | --- | --- |
| `base64` | Event body decoding | MIT OR Apache-2.0 |
| `chrono` | Credential expiration timestamps | MIT OR Apache-2.0 |
| `clap` | Command-line parsing | MIT OR Apache-2.0 |
| `fs4` | Cross-platform file locking | MIT OR Apache-2.0 |
| `futures-util` | Streaming response utilities | MIT OR Apache-2.0 |
| `hostname` | Device Authorization device name | MIT |
| `home` | Cross-platform home-directory fallback | MIT OR Apache-2.0 |
| `keyring` | Platform credential stores | MIT OR Apache-2.0 |
| `keyring-core` | Windows legacy credential migration interface | MIT OR Apache-2.0 |
| `reqwest` | HTTP client with rustls TLS | MIT OR Apache-2.0 |
| `secret-service` | Migration reader for legacy Linux keytar entries | MIT OR Apache-2.0 |
| `serde`, `serde_json` | Wire contracts and JSON | MIT OR Apache-2.0 |
| `tempfile` | Secure same-directory atomic writes | MIT OR Apache-2.0 |
| `terminal_size` | Responsive terminal output | MIT OR Apache-2.0 |
| `thiserror` | Typed errors | MIT OR Apache-2.0 |
| `tokio` | Async runtime and signals | MIT |
| `toml` | Local configuration parsing and serialization | MIT OR Apache-2.0 |
| `unicode-segmentation`, `unicode-width` | Terminal-safe text layout | MIT OR Apache-2.0 |
| `url` | API and redirect URL handling | MIT OR Apache-2.0 |
| `uuid` | Idempotency keys | Apache-2.0 OR MIT |
| `windows-native-keyring-store` | Windows legacy Credential Manager migration | MIT OR Apache-2.0 |

`keyring` selects Apple Keychain, Windows Credential Manager, and Linux
Secret Service backends. `reqwest` selects `rustls`, `rustls-webpki`, and the
Mozilla-derived `webpki-roots` certificate set instead of a native TLS/OpenSSL
dependency.

## Transitive licenses

The locked runtime graph uses permissive licenses, principally MIT,
Apache-2.0, ISC, BSD-3-Clause, Unicode-3.0, Zlib, Unlicense, and
CDLA-Permissive-2.0. A few crates offer additional alternative licenses; the
distribution relies on their permissive option shown by their SPDX expression.
Exact crate versions and checksums are recorded in `Cargo.lock`.

The corresponding license texts and copyright notices are available from each
crate's source distribution in the Cargo registry. The primary common license
texts are:

- Apache License 2.0: <https://www.apache.org/licenses/LICENSE-2.0>
- MIT License: <https://opensource.org/license/mit>
- ISC License: <https://opensource.org/license/isc-license-txt>
- Unicode License v3: <https://www.unicode.org/license.txt>
- Community Data License Agreement Permissive 2.0:
  <https://cdla.dev/permissive-2-0/>

This notice is informational and does not modify any upstream license.
