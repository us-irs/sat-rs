Change Log
=======

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/)
and this project adheres to [Semantic Versioning](http://semver.org/).

# [unreleased]

# [v0.2.2] 2025-05-10

- Bump to `spacepackests` v0.14

# [v0.2.1] 2024-11-15

Increased allowed spacepackets to v0.13

# [v0.2.0] 2024-11-04

Semver bump, due to added features in v0.1.4

# [v0.1.4] 2024-04-24

## Added

- `ResultU16::from_be_bytes`
- `From<u16>` impl for `ResultU16`.
- Optional `defmt` support: `defmt::Format` impl on `ResultU16` if the `defmt` feature is
  activated.

# [v0.1.3] 2024-04-16

Allow `spacepackets` range starting with v0.10 and v0.11.

# [v0.1.2] 2024-02-17

- Bumped `spacepackets` to v0.10.0 for `UnsignedEnum` trait change.

# [v0.1.1] 2024-02-12

- Added missing `#![no_std]` attribute for library
- Fixed unit tests

# [v0.1.0] 2024-02-12

Initial release.

[unreleased]: https://egit.irs.uni-stuttgart.de/rust/sat-rs/compare/satrs-shared-v0.2.2...HEAD
[v0.2.2]: https://egit.irs.uni-stuttgart.de/rust/sat-rs/compare/satrs-shared-v0.2.1...satrs-shared-v0.2.2
