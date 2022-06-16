use postcard::to_stdvec;
use serde::{Deserialize, Serialize};
use zerocopy::byteorder::{I32, U16};
use zerocopy::{AsBytes, FromBytes, NetworkEndian, Unaligned};

#[derive(AsBytes, FromBytes, Unaligned, Debug, Eq, PartialEq)]
#[repr(C, packed)]
struct ZeroCopyTest {
    some_bool: u8,
    some_u16: U16<NetworkEndian>,
    some_i32: I32<NetworkEndian>,
    some_float: [u8; 4],
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct PostcardTest {
    some_bool: u8,
    some_u16: u16,
    some_i32: i32,
    some_float: f32,
}

fn main() {
    let pc_test = PostcardTest {
        some_bool: true as u8,
        some_u16: 0x42,
        some_i32: -200,
        some_float: 7.7_f32,
    };

    let out = to_stdvec(&pc_test).unwrap();
    println!("{:#04x?}", out);

    let sample_hk = ZeroCopyTest {
        some_bool: true as u8,
        some_u16: U16::from(0x42),
        some_i32: I32::from(-200),
        some_float: 7.7_f32.to_ne_bytes(),
    };
    let mut slice = [0; 11];
    sample_hk.write_to(slice.as_mut_slice());
    println!("{:#04x?}", slice);
}
