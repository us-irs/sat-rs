sat-rs example for the STM32F3-Discovery board
=======

This example application shows how the [sat-rs framework](https://egit.irs.uni-stuttgart.de/rust/satrs-launchpad)
can be used on an embedded target. It also shows how a relatively simple OBSW could be built when no
standard runtime is available. It uses [RTIC](https://rtic.rs/1/book/en/) as the concurrency
framework.

The STM32F3-Discovery device was picked because it is a cheap Cortex-M4 based device which is also
used by the [Rust Embedded Book](https://docs.rust-embedded.org/book/intro/hardware.html) and the
[Rust Discovery](https://docs.rust-embedded.org/discovery/f3discovery/) book as an introduction
to embedded Rust.

## Preparing Rust and the repository

Building an application requires the `thumbv7em-none-eabihf` cross-compiler toolchain.
If you have not installed it yet, you can do so with

```sh
rustup target add thumbv7em-none-eabihf
```

A default `.cargo` config file is provided for this project, but needs to be copied to have
the correct name. This is so that the config file can be updated or edited for custom needs
without being tracked by git.

```sh
cp def_config.toml config.toml
```

The configuration file will also set the target so it does not always have to be specified with
the `--target` argument.

## Building

After that, assuming that you have a `.cargo/config.toml` setting the correct build target,
you can simply build the application with

```sh
cargo build
```

## Flashing and Debugging from the command line

TODO

## Debugging with VS Code

The STM32F3-Discovery comes with an on-board ST-Link so all that is required to flash and debug
the board is a Mini-USB cable. The code in this repository was debugged using `openocd`
and the VS Code [`Cortex-Debug` plugin](https://marketplace.visualstudio.com/items?itemName=marus25.cortex-debug).

Some sample configuration files for VS Code were provided as well. You can simply use `Run` and `Debug`
to automatically rebuild and flash your application.

The `tasks.json` and `launch.json` files are generic and you can use them immediately by opening
the folder in VS code or adding it to a workspace.

If you would like to use a custom GDB application, you can specify the gdb binary in the following
configuration variables in your `settings.json`:

- `"cortex-debug.gdbPath"`
- `"cortex-debug.gdbPath.linux"`
- `"cortex-debug.gdbPath.windows"`
- `"cortex-debug.gdbPath.osx"`

## Commanding with Python

When the SW is running on the Discovery board, you can command the MCU via a serial interface,
using COBS encoded CCSDS packets.
 
TODO:
  - How and where to connect serial interface on the MCU
  - How to set up Python venv (or at least strongly recommend it) and install deps
  - How to copy `def_tmtc_conf.json` to `tmtc_conf.json` and adapt it for custom serial port
