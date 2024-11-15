#![allow(dead_code)]

use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::atomic::{AtomicU16, Ordering};
use std::thread;
use zerocopy::{FromBytes, Immutable, IntoBytes, NetworkEndian, Unaligned, U16};

trait FieldDataProvider: Send {
    fn get_data(&self) -> &[u8];
}

struct FixedFieldDataWrapper {
    data: [u8; 8],
}

impl FixedFieldDataWrapper {
    pub fn from_two_u32(p0: u32, p1: u32) -> Self {
        let mut data = [0; 8];
        data[0..4].copy_from_slice(p0.to_be_bytes().as_slice());
        data[4..8].copy_from_slice(p1.to_be_bytes().as_slice());
        Self { data }
    }
}

impl FieldDataProvider for FixedFieldDataWrapper {
    fn get_data(&self) -> &[u8] {
        self.data.as_slice()
    }
}

type FieldDataTraitObj = Box<dyn FieldDataProvider>;

struct ExampleMgmSet {
    mgm_vec: [f32; 3],
    temperature: u16,
}

#[derive(FromBytes, IntoBytes, Immutable, Unaligned)]
#[repr(C)]
struct ExampleMgmSetZc {
    mgm_vec: [u8; 12],
    temperatur: U16<NetworkEndian>,
}

fn main() {
    let (s0, r0): (Sender<FieldDataTraitObj>, Receiver<FieldDataTraitObj>) = bounded(5);
    let data_wrapper = FixedFieldDataWrapper::from_two_u32(2, 3);
    s0.send(Box::new(data_wrapper)).unwrap();
    let jh0 = thread::spawn(move || {
        let data = r0.recv().unwrap();
        let raw = data.get_data();
        println!("Received data {raw:?}");
    });
    let jh1 = thread::spawn(|| {});
    jh0.join().unwrap();
    jh1.join().unwrap();
    //let mut max_val: u16 = u16::MAX;
    //max_val += 1;
    //println!("Max val: {}", max_val);
    let atomic_u16: AtomicU16 = AtomicU16::new(u16::MAX);
    atomic_u16.fetch_add(1, Ordering::SeqCst);
    println!(
        "atomic after overflow: {}",
        atomic_u16.load(Ordering::SeqCst)
    );
}
