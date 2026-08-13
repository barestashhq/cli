# Documentation

| Document | Purpose |
| --- | --- |
| [CLI design](cli-design.md) | Command, output, API, auth, SSE, and terminal behavior contract |
| [Rust migration inventory](rust-migration-inventory.md) | Behaviors extracted from the TypeScript reference before implementation |
| [Native release](release.md) | Verification, target archives, checksums, and publication |

Repository workflow:

```bash
just check
just test
just build
just ci
```
