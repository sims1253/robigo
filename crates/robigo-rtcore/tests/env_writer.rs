//! The environment writer (`clib::EnvWriter`) holds one write permission
//! for a whole builtin call: no isolate starts while it lives, so a
//! `Sys.setenv` of several variables cannot be refused part way. Its own
//! test binary, so that no other test's isolates are counted.

use robigo_rtcore::clib::env_writer;
use robigo_rtcore::{OwnedIsolate, live_isolate_count};
use std::sync::mpsc;
use std::time::Duration;

// (the C library's version is not Miri's)
#[test]
#[cfg_attr(miri, ignore)]
fn no_isolate_starts_while_an_environment_writer_lives() {
    assert_eq!(live_isolate_count(), 0);
    let w = env_writer().expect("one isolate or none: the write is allowed");
    assert!(w.set(b"ROBIGO_ENV_WRITER_TEST", b"1"));
    let (tx, rx) = mpsc::channel();
    let t = std::thread::spawn(move || {
        let iso = OwnedIsolate::new();
        tx.send(()).unwrap();
        drop(iso);
    });
    // the other thread's isolate waits for the writer
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "an isolate started while the writer lived");
    assert!(w.has(b"ROBIGO_ENV_WRITER_TEST"));
    w.unset(b"ROBIGO_ENV_WRITER_TEST");
    assert!(!w.has(b"ROBIGO_ENV_WRITER_TEST"));
    drop(w);
    rx.recv_timeout(Duration::from_secs(30)).expect("the isolate started once the writer was dropped");
    t.join().unwrap();
}
