//! What the borrow checker proves about the API (docs/g3.md §2.2): each
//! case under `tests/ui` must fail to compile, with the error recorded next
//! to it. A missing or different record fails the test (trybuild writes the
//! actual output under the target directory's `wip/`, never into
//! `tests/ui`); regenerate the records with `TRYBUILD=overwrite` after a
//! compiler update, and read the diff.

#[cfg(not(miri))]
#[test]
fn misuse_does_not_compile() {
    trybuild::TestCases::new().compile_fail("tests/ui/*.rs");
}

/// Every case has its committed record: a case without one would fail
/// above as "wip", with less of a hint than this.
#[cfg(not(miri))]
#[test]
fn every_case_has_its_record() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/ui");
    let mut cases = 0;
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "rs") {
            cases += 1;
            assert!(p.with_extension("stderr").is_file(), "{} has no .stderr record", p.display());
        }
    }
    assert_eq!(cases, 8, "the compile-fail cases (update this count with a new case)");
}
