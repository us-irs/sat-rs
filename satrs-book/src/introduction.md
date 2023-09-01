The sat-rs book
======

This book is the primary information resource for the [sat-rs framework](https://egit.irs.uni-stuttgart.de/rust/sat-rs)
in addition to the regular API documentation. It contains the following resources:

1. Architecture informations and consideration which would exceeds the scope of the regular API.
2. General information on how to build On-Board Software and how `sat-rs` can help to fulfill
   the unique requirements of writing software for remote systems.
2. A Getting-Started workshop where a small On-Board Software is built from scratch using
   sat-rs components.

# Introduction

The primary goal of the sat-rs framework is to provide re-usable components
to write on-board software for remote systems like rovers or satellites. It is specifically written
for the special requirements for these systems.

A lot of the architecture and general design considerations are based on the
[FSFW](https://egit.irs.uni-stuttgart.de/fsfw/fsfw) C++ framework which has flight heritage
through the 2 missions [FLP](https://www.irs.uni-stuttgart.de/en/research/satellitetechnology-and-instruments/smallsatelliteprogram/flying-laptop/)
and [EIVE](https://www.irs.uni-stuttgart.de/en/research/satellitetechnology-and-instruments/smallsatelliteprogram/EIVE/).

