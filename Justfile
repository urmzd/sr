default: check

init:
    rustup component add clippy rustfmt
    cargo run -p sr -- init

install:
    cargo install --path crates/cli

build:
    cargo build --workspace

run *ARGS:
    cargo run -p sr -- {{ARGS}}

test:
    cargo test --workspace

# Round-trip sr's offline lock rewrites against the real resolvers.
# Needs cargo/uv/poetry/npm on PATH plus a network; missing tools skip.
conformance:
    cargo test -p sr-core --test lock_conformance -- --ignored --nocapture

lint:
    cargo clippy --workspace -- -D warnings

fmt:
    cargo fmt --all

check-fmt:
    cargo fmt --all -- --check

publish:
    cargo publish -p sr-core --dry-run
    cargo publish -p sr --dry-run

record:
    teasr showme

check: check-fmt lint test

ci: check-fmt lint build test
