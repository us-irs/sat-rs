use std::fmt;
use std::error::Error;
use thiserror::Error;

struct Event {
    event_id: u32
}

trait SystemObject {
    fn get_object_id(&self) -> u32;
    fn initialize(&mut self) -> Result<(), Box<dyn Error>>;
}

fn main() {
    println!("Hello World");
}
