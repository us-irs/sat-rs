#![no_std]
#[cfg(any(feature = "std", test))]
extern crate std;

pub use satrs_mib_codegen::*;
pub mod res_code;
