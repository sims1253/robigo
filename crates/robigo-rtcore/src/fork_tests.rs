//! The process's fork handlers (robigo-heap's `fork.rs`), one test per lock:
//! another thread holds the lock while this one forks, and the child must
//! still start an isolate, write the environment (`Sys.setenv`'s writer)
//! and intern a new symbol. Without the handlers the child inherits the
//! lock held by a thread it does not have, and blocks forever.

#![allow(unsafe_code)]

use crate::OwnedIsolate;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// What the child does after the fork: everything that takes the three
/// locks. Exit status 0 if it all worked.
fn child_work() -> i32 {
    let r = std::panic::catch_unwind(|| {
        let mut iso = OwnedIsolate::new();
        // SAFETY: the child has one thread, this one: no other reads the
        // environment while it writes.
        iso.grant_env_writes(unsafe { crate::clib::EnvWrites::assume() });
        let cx = iso.cx();
        let w = crate::clib::env_writer(cx).expect("an environment writer in the child");
        assert!(w.set(b"ROBIGO_FORK_TEST", b"1"));
        drop(w);
        let _ = cx.install(format!("robigo_fork_test_{}", std::process::id()).as_bytes()).unwrap();
    });
    if r.is_ok() { 0 } else { 1 }
}

/// Fork while another thread holds a lock (`hold` returns its guard), and
/// check that the child finishes `child_work`.
fn fork_while_held<G: 'static>(what: &str, hold: fn() -> G) {
    // the handlers are registered with the process's first heap
    drop(OwnedIsolate::new());
    let (held_tx, held_rx) = mpsc::channel();
    let holder = std::thread::spawn(move || {
        let g = hold();
        held_tx.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        drop(g);
    });
    held_rx.recv().unwrap();
    // SAFETY: the child runs `child_work` and exits without returning into
    // the test harness (`_exit`).
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork");
    if pid == 0 {
        let s = child_work();
        // SAFETY: ends the child at once, running no handler of the parent's.
        unsafe { libc::_exit(s) };
    }
    holder.join().unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut status = 0;
    loop {
        // SAFETY: `pid` is this process's child; `status` is writable.
        let r = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        if r == pid {
            break;
        }
        if Instant::now() > deadline {
            // SAFETY: our own child.
            unsafe { libc::kill(pid, libc::SIGKILL) };
            panic!("{what}: the child forked while another thread held it never finished");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0, "{what}: the child failed (status {status})");
}

#[test]
fn a_child_forked_while_the_live_isolate_lock_is_held_starts_an_isolate() {
    fork_while_held("LIVE_ISOLATES", crate::isolate::live_isolates);
}

#[test]
fn a_child_forked_while_the_environment_lock_is_held_writes_it() {
    fork_while_held("ENV", crate::clib::env_write_lock);
}

#[test]
fn a_child_forked_while_the_environment_lock_is_read_writes_it() {
    fork_while_held("ENV shared", crate::clib::env_read_for_test);
}

#[test]
fn a_child_forked_while_the_symbol_arena_is_locked_interns() {
    fork_while_held("the arena", robigo_heap::hold_arena_lock_for_test);
}
