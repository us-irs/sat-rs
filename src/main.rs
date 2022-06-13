use core::ops::Deref;
use heapless::Vec;
use postcard::{from_bytes, to_vec};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
struct RefStruct<'a> {
    bytes: &'a [u8],
    str_s: &'a str,
}

fn main() {
    let message = "hElLo";
    let bytes = [0x01, 0x10, 0x02, 0x20];
    let output: Vec<u8, 11> = to_vec(&RefStruct {
        bytes: &bytes,
        str_s: message,
    })
    .unwrap();

    assert_eq!(
        &[0x04, 0x01, 0x10, 0x02, 0x20, 0x05, b'h', b'E', b'l', b'L', b'o',],
        output.deref()
    );

    let out: RefStruct = from_bytes(output.deref()).unwrap();
    assert_eq!(
        out,
        RefStruct {
            bytes: &bytes,
            str_s: message,
        }
    );
}
