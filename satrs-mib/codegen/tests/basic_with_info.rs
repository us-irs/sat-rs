//! Basic check which just verifies that everything compiles
use satrs_mib::resultcode;
use satrs_shared::res_code::ResultU16;

#[resultcode(info = "This is a test result where the first parameter is foo")]
const _TEST_RESULT: ResultU16 = ResultU16::new(0, 1);

fn main() {}
