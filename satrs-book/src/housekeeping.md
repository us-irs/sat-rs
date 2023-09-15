# Housekeeping Data

Remote systems like satellites and rovers oftentimes generate data autonomously and periodically.
The most common example for this is temperature or attitude data. Data like this is commonly
referred to as housekeeping data, and is usually one of the most important and most resource heavy
data sources received from a satellite. Standards like the PUS Service 3 make recommendation how to
expose housekeeping data, but the applicability of the interface offered by PUS 3 has proven to be
partially difficult and clunky for modular systems.

First, we are going to list some assumption and requirements about Housekeeping (HK) data:

1. HK data is generated periodically by various system components throughout the
   systems.
2. An autonomous and periodic sampling of that HK data to be stored and sent to Ground is generally
   required. A minimum interface consists of requesting a one-shot sample of HK, enabling and
   disabling the periodic autonomous generation of samples and modifying the collection interval
   of the periodic autonomous generation.
3. HK data often needs to be shared to other software components. For example, a thermal controller
   wants to read the data samples of all sensor components.

A commonly required way to model HK data in a clean way is also to group related HK data into sets,
which can then dumped via a similar interface.

TODO: Write down `sat-rs` recommendations how to expose and work with HK data.
