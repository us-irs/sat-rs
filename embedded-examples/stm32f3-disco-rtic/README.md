sat-rs example for the STM32F3-Discovery board
=======

This example application shows how the [sat-rs library](https://egit.irs.uni-stuttgart.de/rust/sat-rs)
can be used on an embedded target.
It also shows how a relatively simple OBSW could be built when no standard runtime is available.
It uses [RTIC](https://rtic.rs/2/book/en/) as the concurrency framework and the
[defmt](https://defmt.ferrous-systems.com/) framework for logging.

The STM32F3-Discovery device was picked because it is a cheap Cortex-M4 based device which is also
used by the [Rust Embedded Book](https://docs.rust-embedded.org/book/intro/hardware.html) and the
[Rust Discovery](https://docs.rust-embedded.org/discovery/f3discovery/) book as an introduction
to embedded Rust.

## Pre-Requisites

Make sure the following tools are installed:

1. [`probe-rs`](https://probe.rs/): Application used to flash and debug the MCU.
2. Optional and recommended: [VS Code](https://code.visualstudio.com/) with
   [probe-rs plugin](https://marketplace.visualstudio.com/items?itemName=probe-rs.probe-rs-debugger)
   for debugging.

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

## Flashing from the command line

You can flash the application from the command line using `probe-rs`:

```sh
probe-rs run --chip STM32F303VCTx
```

## Debugging with VS Code

The STM32F3-Discovery comes with an on-board ST-Link so all that is required to flash and debug
the board is a Mini-USB cable. The code in this repository was debugged using [`probe-rs`](https://probe.rs/docs/tools/debuggerA)
and the VS Code [`probe-rs` plugin](https://marketplace.visualstudio.com/items?itemName=probe-rs.probe-rs-debugger).
Make sure to install this plugin first.

Sample configuration files are provided inside the `vscode` folder.
Use `cp vscode .vscode -r` to use them for your project.

Some sample configuration files for VS Code were provided as well. You can simply use `Run` and `Debug`
to automatically rebuild and flash your application.

The `tasks.json` and `launch.json` files are generic and you can use them immediately by opening
the folder in VS code or adding it to a workspace.

## Commanding with Python

When the SW is running on the Discovery board, you can command the MCU via a serial interface,
using COBS encoded PUS packets.

It is recommended to use a virtual environment to do this. To set up one in the command line,
you can use `python3 -m venv venv` on Unix systems or `py -m venv venv` on Windows systems.
After doing this, you can check the [venv tutorial](https://docs.python.org/3/tutorial/venv.html)
on how to activate the environment and then use the following command to install the required
dependency:

```sh
pip install -r requirements.txt
```

The packets are exchanged using a dedicated serial interface. You can use any generic USB-to-UART
converter device with the TX pin connected to the PA3 pin and the RX pin connected to the PA2 pin.

A default configuration file for the python application is provided and can be used by running

```sh
cp def_tmtc_conf.json tmtc_conf.json
```

After that, you can for example send a ping to the MCU using the following command

```sh
./main.py -p /ping
```

You can configure the blinky frequency using

```sh
./main.py -p /change_blink_freq
```

All these commands will package a PUS telecommand which will be sent to the MCU using the COBS
format as the packet framing format.
