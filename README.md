sat-rs launchpad
=========

This is the prototyping repository for the initial version of a
Rust Flight Software Framework.

Currently, it contains the following major subcrates:

1. The [`spacepackets`](https://egit.irs.uni-stuttgart.de/rust/spacepackets) crate which contains
   basic ECSS and CCSDS packet protocol implementations.
2. The [`satrs-core`](https://egit.irs.uni-stuttgart.de/rust/satrs-launchpad/src/branch/main/satrs-core)
   crate containing the core components of sat-rs
3. The [`satrs-example`](https://egit.irs.uni-stuttgart.de/rust/satrs-launchpad/src/branch/main/satrs-example)
   crate which shows a simple example on-board software using various sat-rs components.
