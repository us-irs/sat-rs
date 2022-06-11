//! This module contains the core components of the Flight Software Rust Crate (FSRC) collection
//! This includes components to perform the following tasks
//!
//! 1. [Object Management][objects]
//! 2. [Task Scheduling][executable]
//! 3. [Events][event] and [event management][event_man]
//! 4. [Pre-Allocated memory pools][pool]
pub mod event_man;
pub mod events;
pub mod executable;
pub mod objects;
pub mod pool;
