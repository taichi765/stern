use trybuild::TestCases;

#[test]
fn foreground_executor_contracts() {
    let t = TestCases::new();
    t.compile_fail("tests/trybuild/slint_executor.rs");
    t.compile_fail("tests/trybuild/smol_executor.rs");
}
