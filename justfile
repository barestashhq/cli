set shell := ["sh", "-eu", "-c"]
set default-list
set minimum-version := "1.55.0"

[doc('Fetch the locked Rust dependencies')]
[group('setup')]
install:
    cargo fetch --locked

[doc('Run formatting, Clippy, and all tests')]
[group('quality')]
check: format-check lint test

[doc('Format Rust source files')]
[group('quality')]
format:
    cargo fmt --all

[doc('Verify Rust source formatting')]
[group('quality')]
format-check:
    cargo fmt --all --check

[doc('Run Clippy with warnings denied')]
[group('quality')]
lint:
    cargo clippy --all-targets --all-features -- -D warnings

[doc('Run all Rust tests')]
[group('quality')]
test:
    cargo test --all

[doc('Build the native release binary')]
[group('quality')]
build:
    cargo build --release --locked

[doc('Verify the Cargo package allowlist and build the release artifact locally')]
[group('quality')]
package: build
    package_files="$(cargo package --locked --allow-dirty --list)"; \
      printf '%s\n' "$package_files"; \
      if printf '%s\n' "$package_files" | grep -Eq '(^|/)(node_modules|\.pnpm-store|coverage|dist)(/|$)|(^|/)(package\.json|pnpm-lock\.yaml|pnpm-workspace\.yaml)$'; then \
        echo "Cargo package unexpectedly contains a Node.js or generated artifact." >&2; \
        exit 1; \
      fi
    ./target/release/barestash --version

[doc('Verify justfile formatting')]
[group('quality')]
[private]
_check-justfile:
    just --fmt --check

[doc('Mirror the CI quality and release build gates')]
[group('quality')]
ci: _check-justfile check package

[doc('Alias for the complete pre-release gate')]
[group('quality')]
ci-full: ci

[doc('Run the native Barestash CLI from source')]
[group('cli')]
[positional-arguments]
barestash *args:
    cargo run --quiet -- "$@"
