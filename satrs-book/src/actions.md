# Working with Actions

Space systems generally need to be commanded regularly. This can include commands periodically
required to ensure a healthy system, or commands to reach the mission goals.

These commands can be modelled using the concept of Actions. the ECSS PUS standard also provides
the PUS service 8 for actions, but provides few concrete subservices and specification on how
action commanding could look like.

`sat-rs` proposes two recommended ways to perform action commanding:

1. Target ID and Action ID based. The target ID is a 32-bit unsigned ID for an OBSW object entity
   which can also accept Actions. The action ID is a 32-bit unsigned ID for each action that a
   target is able to perform.
2. Target ID and Action String based. The target ID is the same as in the first proposal, but
   the unique action is identified by a string.

The library provides an `ActionRequest` abstraction to model both of these cases.

## Commanding with ECSS PUS 8

`sat-rs` provides a generic ECSS PUS 8 action command handler. This handler can convert PUS 8
telecommands which use the commanding scheme 1 explained above to an `ActionRequest` which is
then forwarded to the target specified by the Target ID.

There are 3 requirements for the PUS 8 telecommand:

1. The subservice 128 must be used
2. Bytes 0 to 4 of application data must contain the target ID in `u32` big endian format.
3. Bytes 4 to 8 of application data must contain the action ID in `u32` big endian format.
4. The rest of the application data are assumed to be command specific additional parameters. They
   will be added to an IPC store and the corresponding store address will be sent as part of the
   `ActionRequest`.

## Sending back telemetry

There are some cases where the regular verification provided by PUS in response to PUS action
commands is not sufficient and some additional telemetry needs to be sent to ground. In that
case, it is recommended to chose some custom subservice for action TM data and then send the
telemetry using the same scheme as shown above, where the first 8 bytes of the application
data is reserved for the target ID and action ID.

