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

/// Every case has its committed record, and every record its case: a case
/// without one would fail above as "wip", with less of a hint than this,
/// and trybuild never reads a record whose case is gone.
#[cfg(not(miri))]
#[test]
fn every_case_has_its_record() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/ui");
    let mut cases = 0;
    for e in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("reading {}: {e}", dir.display())) {
        let p = e.unwrap_or_else(|e| panic!("reading {}: {e}", dir.display())).path();
        match p.extension().and_then(|x| x.to_str()) {
            Some("rs") => {
                cases += 1;
                assert!(p.with_extension("stderr").is_file(), "{} has no .stderr record", p.display());
            }
            Some("stderr") => assert!(p.with_extension("rs").is_file(), "{} has no .rs case", p.display()),
            _ => {}
        }
    }
    assert_eq!(cases, 8, "the compile-fail cases (update this count with a new case)");
}
