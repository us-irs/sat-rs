//! Basic check which just verifies that everything compiles
use satrs_core::res_code::ResultU16;
use satrs_macros::*;

#[resultcode(info = "This is a test result where the first parameter is foo")]
const _TEST_RESULT: ResultU16 = ResultU16::const_new(0, 1);

fn main() {}
