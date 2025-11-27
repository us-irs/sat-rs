all: check build embedded test check-fmt clippy docs

check:
  cargo check
  cargo check -p satrs-example --no-default-features

build:
  cargo build

test:
  cargo nextest run --all-features
  cargo test --doc --all-features

embedded:
  cargo check -p satrs --target=thumbv7em-none-eabihf --no-default-features

check-fmt:
  cargo fmt --all -- --check

fmt:
  cargo fmt --all

clippy:
  cargo clippy -- -D warnings

docs-satrs:
  RUSTDOCFLAGS="--cfg docsrs --generate-link-to-definition -Z unstable-options" cargo +nightly doc -p satrs --all-features

docs: docs-satrs
