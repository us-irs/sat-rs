//! # Core components of the Flight Software Rust Crate (FSRC) collection.
//!
//! This includes components to perform the following tasks
//!
//! 1. Object Management with the [objects] module
//! 2. Task schedule with the [executable] module
//! 3. Events with the [events] module and event management with the [event_man] module
//! 4. Pre-Allocated memory pools with the [pool] module
pub mod event_man;
pub mod events;
pub mod executable;
pub mod objects;
pub mod pool;
