//! Forks that overlap the process's first isolate start (Sol's case for the
//! fork handlers' callbacks): one thread starts the first isolate while this
//! one forks again and again, and every child must start an isolate and
//! write the environment. Its own test binary, so that the isolate the
//! other thread starts is the process's first.
//!
//! Coverage, not proof of overlap: the forks may land before the other
//! thread's start gets far. The window this case is about is closed by
//! construction instead: robigo-rtcore's constructor publishes its fork
//! locks before `main`, and `with_options` publishes them again before
//! `Registration::new` takes the first lock (`isolate.rs`), so no
//! interleaving lets a fork find one of them held before the handlers know
//! it. The sibling tests in `src/fork_tests.rs` observe their overlap.

#[test]
#[cfg(all(target_os = "linux", not(miri)))]
fn forks_overlapping_the_first_isolate_start_give_working_children() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};
    // a fork whose handlers hang never returns, before any deadline below
    // could run: abort the process instead
    // disarmed when the test ends, a failing one too (its guard drops)
    struct Disarm(Arc<AtomicBool>);
    impl Drop for Disarm {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let done = Arc::new(AtomicBool::new(false));
    {
        let done = Arc::clone(&done);
        std::thread::spawn(move || {
            let until = Instant::now() + Duration::from_secs(90);
            // (the flag read after the last sleep too)
            while !done.load(Ordering::SeqCst) {
                if Instant::now() >= until {
                    // raw write(2), and only when it cannot block: std's
                    // stderr lock, or a full stderr pipe, may be what is stuck
                    let msg = b"fork_first_isolate: no progress in 90 s (a fork or its handlers hung)\n";
                    let mut p = libc::pollfd { fd: 2, events: libc::POLLOUT, revents: 0 };
                    // SAFETY: one pollfd, no wait; a live buffer to fd 2.
                    unsafe {
                        if libc::poll(&mut p, 1, 0) == 1 && p.revents & libc::POLLOUT != 0 {
                            libc::write(2, msg.as_ptr().cast(), msg.len());
                        }
                    }
                    std::process::abort();
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
    }
    let _disarm = Disarm(Arc::clone(&done));
    let starter = std::thread::spawn(|| {
        let mut iso = robigo_rtcore::OwnedIsolate::new();
        let cx = iso.cx();
        for i in 0..2000 {
            let _ = cx.install(format!("fork_first_isolate_{i}").as_bytes()).unwrap();
        }
    });
    let mut children = Vec::new();
    for k in 0..40 {
        // SAFETY: the child runs the code below and exits without returning
        // into the test harness.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork {k} failed: {}", std::io::Error::last_os_error());
        if pid == 0 {
            let ok = std::panic::catch_unwind(|| {
                let mut iso = robigo_rtcore::OwnedIsolate::new();
                // SAFETY: the child has one thread, this one.
                iso.grant_env_writes(unsafe { robigo_rtcore::clib::EnvWrites::assume() });
                let cx = iso.cx();
                // the child's count still includes the starter's isolate: on
                // a glibc older than MIN_GLIBC that is the writer's
                // documented refusal, on a newer one a write
                match robigo_rtcore::clib::env_writer(cx) {
                    Ok(w) => assert!(w.set(b"ROBIGO_FORK_FIRST", b"1"), "child {k}: setenv"),
                    Err(robigo_rtcore::clib::EnvWriteRefused::SeveralIsolates)
                        if robigo_rtcore::clib::glibc_version() < robigo_rtcore::clib::MIN_GLIBC => {}
                    Err(e) => panic!("child {k}: the environment writer: {e:?}"),
                }
            })
            .is_ok();
            // SAFETY: ends the child at once.
            unsafe { libc::_exit(if ok { 0 } else { 1 }) };
        }
        children.push((k, pid));
    }
    starter.join().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    for (k, pid) in children {
        let mut status = 0;
        loop {
            // SAFETY: our own child; `status` is writable.
            let r = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
            if r == pid {
                break;
            }
            assert!(r == 0, "child {k}: waitpid failed: {}", std::io::Error::last_os_error());
            if Instant::now() > deadline {
                // SAFETY: our own child, killed and reaped (again after a
                // signal).
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                    while libc::waitpid(pid, &mut status, 0) == -1
                        && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR)
                    {}
                }
                panic!("child {k}, forked during the first isolate's start, never finished");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0, "child {k} failed (status {status})");
    }
}
