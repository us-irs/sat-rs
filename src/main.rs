use launchpad::core::events::{Event, Severity};
use std::sync::mpsc::channel;
use std::thread;


fn main() {
    basic_event_test();
}

fn basic_event_test() {
    let test_event_0 = Event::new(Severity::INFO, 0, 0);
    let (event_sender, event_receiver) = channel();
    let t0_sender = event_sender.clone();
    let t0 = thread::spawn(move || {
        t0_sender
            .send(test_event_0.unwrap())
            .expect("Sending event from t0 failed");
    });

    let test_event_1 = Event::new(Severity::MEDIUM, 1, 8);
    let t1 = thread::spawn(move || {
        event_sender
            .send(test_event_1.unwrap())
            .expect("Sending event from t1 failed");
    });

    let mut event_cntr = 0;
    while event_cntr < 2 {
        if let Ok(event) = event_receiver.recv() {
            println!("Received event {:?}", event);
            event_cntr += 1;
        }
    }
    t0.join().unwrap();
    t1.join().unwrap();
}
