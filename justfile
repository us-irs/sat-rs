all: check embedded test fmt clippy docs

check:
  cargo check
  cargo check -p satrs-example --no-default-features

test:
  cargo nextest run --all-features
  cargo test --doc --all-features

embedded:
  cargo check -p satrs --target=thumbv7em-none-eabihf --no-default-features

fmt:
  cargo fmt --all -- --check

clippy:
  cargo clippy -- -D warnings

docs:
  cargo +nightly doc --all-features --config 'build.rustdocflags=["--cfg", "docs_rs"]'
