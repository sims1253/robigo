//! The process's first environment write against a fork between its
//! handlers (Sol r5 S10). The writer's first read of the glibc version
//! takes `FORK` (a lazy initializer: robigo-heap's `once_excluding_fork`),
//! so it must happen before the writer takes `LIVE_ISOLATES`, which a fork's
//! `prepare` takes after `FORK`. Its own test binary, so that the glibc
//! version is still unread.
//!
//! One thread holds `FORK` as a fork's `prepare` would, while this one
//! starts the first write. Then the holder asks for `LIVE_ISOLATES`, as
//! `prepare` would next.
//! - In the right order, the writer waits for `FORK` without holding
//!   `LIVE_ISOLATES`, so the holder gets it.
//! - In the wrong order, the writer holds `LIVE_ISOLATES` while it waits
//!   for `FORK`, and the holder cannot get it: the test sees the deadlock
//!   (after 2 s) instead of hanging, because the holder then lets `FORK` go.

#[test]
#[cfg(all(target_os = "linux", not(miri)))]
fn the_first_environment_write_does_not_hold_the_live_isolates_while_it_waits_for_fork() {
    use std::sync::mpsc;
    use std::time::Duration;
    let mut iso = robigo_rtcore::OwnedIsolate::new();
    // SAFETY: this test's threads read the environment only through the
    // writer's own calls.
    iso.grant_env_writes(unsafe { robigo_rtcore::clib::EnvWrites::assume() });
    let (held_tx, held_rx) = mpsc::channel();
    let holder = std::thread::spawn(move || {
        // SAFETY: this thread forks nothing and initializes nothing while it
        // holds the guard, and drops it here.
        let fork = unsafe { robigo_heap::hold_fork_lock_for_test() };
        held_tx.send(()).unwrap();
        // the writer is now inside env_writer, waiting for `FORK`
        std::thread::sleep(Duration::from_millis(200));
        // `prepare`'s next lock: asked for on another thread, so a writer
        // holding it cannot hang this one
        let (got_tx, got_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = robigo_rtcore::live_isolate_count();
            let _ = got_tx.send(());
        });
        let got = got_rx.recv_timeout(Duration::from_secs(2)).is_ok();
        drop(fork);
        got
    });
    held_rx.recv().unwrap();
    let cx = iso.cx();
    let w = robigo_rtcore::clib::env_writer(cx);
    let wrote = match w {
        Ok(w) => w.set(b"ROBIGO_FORK_ENV_WRITE", b"1"),
        Err(e) => panic!("the writer: {e:?}"),
    };
    assert!(wrote, "setenv");
    assert!(
        holder.join().unwrap(),
        "the first environment write held LIVE_ISOLATES while it waited for FORK (a fork's prepare would deadlock)"
    );
}
