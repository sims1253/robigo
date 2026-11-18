//! Forks that overlap the process's first isolate start (Sol's case for the
//! fork handlers' callbacks): one thread starts the first isolate while this
//! one forks again and again, and every child must start an isolate and
//! write the environment. Its own test binary, so that the isolate the
//! other thread starts is the process's first.

#[test]
#[cfg(all(target_os = "linux", not(miri)))]
fn forks_overlapping_the_first_isolate_start_give_working_children() {
    use std::time::{Duration, Instant};
    let starter = std::thread::spawn(|| {
        let mut iso = robigo_rtcore::OwnedIsolate::new();
        let cx = iso.cx();
        for i in 0..2000 {
            let _ = cx.install(format!("fork_first_isolate_{i}").as_bytes()).unwrap();
        }
    });
    let mut children = Vec::new();
    for _ in 0..40 {
        // SAFETY: the child runs the code below and exits without returning
        // into the test harness.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork");
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
                    Ok(w) => assert!(w.set(b"ROBIGO_FORK_FIRST", b"1")),
                    Err(robigo_rtcore::clib::EnvWriteRefused::SeveralIsolates)
                        if robigo_rtcore::clib::glibc_version() < robigo_rtcore::clib::MIN_GLIBC => {}
                    Err(e) => panic!("{e:?}"),
                }
            })
            .is_ok();
            // SAFETY: ends the child at once.
            unsafe { libc::_exit(if ok { 0 } else { 1 }) };
        }
        children.push(pid);
    }
    starter.join().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    for pid in children {
        let mut status = 0;
        loop {
            // SAFETY: our own child; `status` is writable.
            let r = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
            if r == pid {
                break;
            }
            if Instant::now() > deadline {
                // SAFETY: our own child.
                unsafe { libc::kill(pid, libc::SIGKILL) };
                panic!("a child forked during the first isolate's start never finished");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0, "a child failed (status {status})");
    }
}
