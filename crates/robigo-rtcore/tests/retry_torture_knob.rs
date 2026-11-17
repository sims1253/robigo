//! [`robigo_rtcore::Cx::set_retry_torture`] refuses the mode while E9's
//! knob is set (`ROBIGO_FAIL_ALLOC_AT`, read when an isolate is made): the
//! mode would take the knob over. Its own test binary, so that the knob
//! reaches no other test (as tests/alias_alloc.rs does).

#![cfg(feature = "retry-torture")]

#[test]
#[should_panic(expected = "ROBIGO_RETRY_TORTURE and ROBIGO_FAIL_ALLOC_AT both use E9's knob: set one")]
fn the_mode_is_refused_while_fail_alloc_at_is_set() {
    // SAFETY: this binary's one test sets it before any isolate is made,
    // and nothing else reads the environment meanwhile.
    unsafe { std::env::set_var("ROBIGO_FAIL_ALLOC_AT", "5") };
    let mut iso = robigo_rtcore::OwnedIsolate::new();
    iso.cx().set_retry_torture(true);
}
