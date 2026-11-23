//! A core does not hold its torture because the environment asks for it:
//! `ROBIGO_TORTURE_AFTER_START` is the arming embedder's to act on
//! (robigo-rt's front end holds, then arms). A core made with the variable
//! set and no explicit hold tortures from its first collection. (A test of
//! its own, in a binary of its own: it sets the process's environment.)

#[test]
fn the_environment_alone_holds_nothing() {
    // SAFETY: this test binary runs this one test, on one thread, and sets
    // the environment before any other code reads it.
    #[allow(unsafe_code)]
    unsafe {
        std::env::set_var("ROBIGO_TORTURE_AFTER_START", "1");
        std::env::set_var("ROBIGO_GCTORTURE", "1");
        std::env::remove_var("R_GCTORTURE");
    }
    let c = robigo_heap::Core::new();
    assert!(!c.torture_held(), "the core held its torture on the environment's word");
    assert!(c.torture(), "gctorture is on from the start");
}
