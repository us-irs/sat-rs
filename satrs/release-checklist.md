Checklist for new releases
=======

# Pre-Release

1. Make sure any new modules are documented sufficiently enough and check docs by running `docs.sh`.
2. Bump version specifier in `Cargo.toml`.
3. Update `CHANGELOG.md`: Convert `unreleased` section into version section with date and add new
   `unreleased` section.
4. Run `cargo test --all-features` or `cargo nextest r --all-features` and `cargo test --doc`.
5. Run `cargo fmt` and `cargo clippy`. Check `cargo msrv` against MSRV in `Cargo.toml`.
6. Wait for CI/CD results for EGit and Github. These also check cross-compilation for bare-metal
   targets.

# Release

1. `cargo publish`

# Post-Release

1. Create a new release on `EGit` with the name `satrs-<version>`.

