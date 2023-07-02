#[test]
fn tests() {
    let t = trybuild::TestCases::new();
    t.pass("tests/basic.rs");
    t.pass("tests/basic_with_info.rs");
    t.pass("tests/verify_gen_struct.rs");
    //t.pass("tests/group_in_enum.rs");
}
