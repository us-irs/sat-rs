use satrs_mib::res_code::ResultU16Info;
use satrs_mib::resultcode;
use satrs_shared::res_code::ResultU16;

#[resultcode(info = "This is a test result where the first parameter is foo")]
const TEST_RESULT: ResultU16 = ResultU16::new(0, 1);
// Create named reference of auto-generated struct, which can be used by IDEs etc.
const TEST_RESULT_EXT_REF: &ResultU16Info = &TEST_RESULT_EXT;

fn main() {
    assert_eq!(TEST_RESULT_EXT.name, "TEST_RESULT");
    assert_eq!(TEST_RESULT_EXT.result, &TEST_RESULT);
    assert_eq!(
        TEST_RESULT_EXT.info,
        "This is a test result where the first parameter is foo"
    );
    assert_eq!(TEST_RESULT_EXT_REF.name, "TEST_RESULT");
}
