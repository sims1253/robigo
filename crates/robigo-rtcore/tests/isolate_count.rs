//! The live-isolate count (`isolate::LIVE_ISOLATES`), which the
//! environment writer's rule reads: an isolate whose construction panics
//! is not left counted. Its own test binary, so that no other test's
//! isolates change the count meanwhile.

use robigo_rtcore::{Options, OwnedIsolate, live_isolate_count};

#[test]
fn an_isolate_whose_construction_panics_is_not_left_counted() {
    assert_eq!(live_isolate_count(), 0);
    {
        let _a = OwnedIsolate::new();
        assert_eq!(live_isolate_count(), 1);
    }
    assert_eq!(live_isolate_count(), 0);
    // a vector memory limit below the heap's size: with_options panics
    // after the isolate was counted
    let r =
        std::panic::catch_unwind(|| OwnedIsolate::with_options(Options { max_vsize: Some(0), ..Options::default() }));
    assert!(r.is_err(), "a limit of 0 was accepted");
    assert_eq!(live_isolate_count(), 0);
}
