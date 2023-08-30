# Communication with sat-rs based software

Communication is a huge topic for satellites. These systems are usually not (directly) connected
to the internet and only have 1-2 communication links during nominal operation. However, most
satellites have internet access during development cycle. There are various standards provided by
CCSDS and ECSS which can be useful to determine how to communicate with the satellite and the
primary On-Board Software.

# Application layer

Current communication with satellite systems is usually packet based. For example, the CCSDS space
packet standard only specifies a 6 byte header with at least 1 byte payload. The PUS packet
standard is a subset of the space packet standard which adds some fields and a 16 bit CRC, but
it is still centered around small packets. `sat-rs` provides support for these ECSS and CCSDS
standards to also attempts to fill the gap to the internet protocol by providing the following
components.

1. UDP TMTC Server. UDP is already packet based which makes it an excellent fit for exchanging
   space packets.
2. TCP TMTC Server. This is a stream based protocol, so the server uses the COBS framing protocol
   to always deliver complete packets.

# Working with telemetry and telecommands (TMTC)

The commands sent to a space system are commonly called telecommands (TC) while the data received
from it are called telemetry (TM). Keeping in mind the previous section, the concept of a TC source
and a TM sink can be applied to most satellites. The TM sink is the one entity where all generated
telemetry arrives in real-time. The most important task of the TM sink usually is to send all
arriving telemetry to the ground segment of a satellite mission immediately. Another important
task might be to store all arriving telemetry persistently. This is especially important for
space systems which do not have permanent contact like low-earth-orbit (LEO) satellites.

The most important task of a TC source is to deliver the telecommands to the correct recipients.
For modern component oriented software using message passing, this usually includes staged
demultiplexing components to determine where a command needs to be sent.

# Low-level protocols and the bridge to the communcation subsystem

Many satellite systems usually use the lower levels of the OSI layer in addition to the application
layer covered by the PUS standard or the CCSDS space packets standard. This oftentimes requires
special hardware like dedicated FPGAs to handle forward error correction fast enough. `sat-rs`
might provide components to handle standard like the Unified Space Data Link Standard (USLP) in
software but most of the time the handling of communication is performed through custom
software and hardware. Still, connecting this custom software and hardware can mostly be done
by using the concept of TC sources and TM sinks mentioned previously.

