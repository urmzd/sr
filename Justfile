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
