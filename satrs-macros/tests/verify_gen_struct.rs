use satrs_core::resultcode::ResultU16;
use satrs_macros::*;

#[result(info = "This is a test result where the first parameter is foo")]
const TEST_RESULT: ResultU16 = ResultU16::const_new(0, 1);

fn main() {}
