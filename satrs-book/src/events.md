# Events

Events can be an extremely important mechanism used for remote systems to monitor unexpected
or expected anomalies and events occuring on these systems. They are oftentimes tied to
Fault Detection, Isolation and Recovery (FDIR) operations, which need to happen autonomously.

Events can also be used as a convenient Inter-Process Communication (IPC) mechansism, which is
also observable for the Ground segment. The PUS Service 5 standardizes how the ground interface
for events might look like, but does not specify how other software components might react
to those events. There is the PUS Service 19, which might be used for that purpose, but the
event components recommended by this framework do not really need this service.

The following images shows how the flow of events could look like in a system where components
can generate events, and where other system components might be interested in those events:

![Event flow](images/event_man_arch.png)
