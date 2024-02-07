# sat-rs Example Application

The `sat-rs` framework includes a monolithic example application which can be found inside
the [`satrs-example`](https://egit.irs.uni-stuttgart.de/rust/sat-rs/src/branch/main/satrs-example)
subdirectory of the repository. The primary purpose of this example application is to show how
the various components of the sat-rs framework could be used as part of a larger on-board
software application.

## Structure of the example project

The example project contains components which could also be expected to be part of a production
On-Board Software.

1. A UDP and TCP server to receive telecommands and poll telemetry from. This might be an optional
   component for an OBSW which is only used during the development phase on ground. The TCP
   server parses space packets by using the CCSDS space packet ID as the packet start delimiter.
2. A PUS service stack which exposes some functionality conformant with the ECSS PUS service. This
   currently includes the following services:
   - Service 1 for telecommand verification.
   - Service 3 for housekeeping telemetry handling.
   - Service 5 for management and downlink of on-board events.
   - Service 8 for handling on-board actions.
   - Service 11 for scheduling telecommands to be released at a specific time.
   - Service 17 for test purposes (pings)
3. An event manager component which handles the event IPC mechanism.
4. A TC source component which demultiplexes and routes telecommands based on parameters like
   packet APID or PUS service and subservice type.
5. A TM sink sink component which is the target of all sent telemetry and sends it to downlink
   handlers like the UDP and TCP server.
6. An AOCS example task which can also process some PUS commands.
