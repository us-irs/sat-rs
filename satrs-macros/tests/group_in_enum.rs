use satrs_core::resultcode::ResultU16;
use satrs_macros::*;

pub enum GroupIds {
    Group0 = 0,
    Group1 = 1,
}

#[result(info = "This is a test result where the first parameter is foo")]
const TEST_RESULT: ResultU16 = ResultU16::const_new(GroupIds::Group0 as u8, 1);

fn main() {}
