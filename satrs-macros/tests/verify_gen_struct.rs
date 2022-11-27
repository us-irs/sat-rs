use satrs_core::resultcode::{ResultU16, ResultU16Ext};
use satrs_macros::*;

#[resultcode(info = "This is a test result where the first parameter is foo")]
const TEST_RESULT: ResultU16 = ResultU16::const_new(0, 1);
// Create named reference of auto-generated struct, which can be used by IDEs etc.
const TEST_RESULT_EXT_REF: &ResultU16Ext = &TEST_RESULT_EXT;

fn main() {
    assert_eq!(TEST_RESULT_EXT.name, "TEST_RESULT");
    assert_eq!(TEST_RESULT_EXT.result, &TEST_RESULT);
    assert_eq!(
        TEST_RESULT_EXT.info,
        "This is a test result where the first parameter is foo"
    );
    assert_eq!(TEST_RESULT_EXT_REF.name, "TEST_RESULT");
}
