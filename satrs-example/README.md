sat-rs example
======

This crate contains an example application which simulates an on-board software.
It uses various components provided by the sat-rs framework to do this. As such, it shows how
a more complex real on-board software could be built from these components. It is recommended to
read the dedicated
[example chapters](https://absatsw.irs.uni-stuttgart.de/projects/sat-rs/book/example.html) inside
the sat-rs book.

The application opens a UDP and a TCP server on port 7301 to receive telecommands.

You can run the application using `cargo run`.

# Features

The example has the `heap_tmtc` feature which is enabled by default. With this feature enabled,
TMTC packets are exchanged using the heap as the backing memory instead of pre-allocated static
stores.

You can run the application without this feature using

```sh
cargo run --no-default-features
```

# Interacting with the sat-rs example

## Simple Client

The `simpleclient` binary target sends a
ping telecommand and then verifies the telemetry generated by the example application.
It can be run like this:

```rs
cargo run --bin simpleclient
```

This repository also contains a more complex client using the
[Python tmtccmd](https://github.com/robamu-org/tmtccmd) module.

## <a id="tmtccmd"></a> Using the tmtccmd Python client

The python client requires a valid installation of the
[tmtccmd package](https://github.com/robamu-org/tmtccmd).

It is recommended to use a virtual environment to do this. To set up one in the command line,
you can use `python3 -m venv venv` on Unix systems or `py -m venv venv` on Windows systems.
After doing this, you can check the [venv tutorial](https://docs.python.org/3/tutorial/venv.html)
on how to activate the environment and then use the following command to install the required
dependency interactively:

```sh
pip install -e .
```

Alternatively, if you would like to use the GUI functionality provided by `tmtccmd`, you can also
install it manually with

```sh
pip install -e .
pip install tmtccmd[gui]
```

After setting up the dependencies, you can simply run the `main.py` script to send commands
to the OBSW example and to view and handle incoming telemetry. The script and the `tmtccmd`
framework it uses allow to easily add and expose additional telecommand and telemetry handling
as Python code. For example, you can use the following command to send a ping like done with
the `simpleclient`:

```sh
./main.py -p /test/ping
```

You can also simply call the script without any arguments to view the command tree.

## Adding the mini simulator application

This example application features a few device handlers. The
[`satrs-minisim`](https://egit.irs.uni-stuttgart.de/rust/sat-rs/src/branch/main/satrs-minisim)
can be used to simulate the physical devices managed by these device handlers.

The example application will attempt communication with the mini simulator on UDP port 7303.
If this works, the device handlers will use communication interfaces dedicated to the communication
with the mini simulator. Otherwise, they will be replaced by dummy interfaces which either
return constant values or behave like ideal devices.

In summary, you can use the following command command to run the mini-simulator first:

```sh
cargo run -p satrs-minisim
```

and then start the example using `cargo run -p satrs-example`.
