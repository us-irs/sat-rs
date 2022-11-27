use satrs_core::resultcode::ResultU16;
use satrs_macros::*;

#[result]
const TEST_RESULT: ResultU16 = ResultU16::const_new(0, 1);

fn main() {}
