//! What the borrow checker proves about the API (docs/g3.md §2.2): each
//! case under `tests/ui` must fail to compile, with the error recorded next
//! to it. Regenerate the records with `TRYBUILD=overwrite` after a compiler
//! update, and read the diff.

#[cfg(not(miri))]
#[test]
fn misuse_does_not_compile() {
    trybuild::TestCases::new().compile_fail("tests/ui/*.rs");
}
