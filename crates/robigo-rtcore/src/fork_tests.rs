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
        // the two locks of this crate, taken directly: free in the child on
        // any glibc (the writer below refuses before it takes `ENV` when an
        // older glibc counts several isolates)
        drop(crate::isolate::live_isolates());
        drop(crate::clib::env_write_lock());
        let mut iso = OwnedIsolate::new();
        // SAFETY: the child has one thread, this one: no other reads the
        // environment while it writes.
        iso.grant_env_writes(unsafe { crate::clib::EnvWrites::assume() });
        let cx = iso.cx();
        // other tests' isolates still count in the child: on a glibc older
        // than MIN_GLIBC that is the writer's documented refusal
        match crate::clib::env_writer(cx) {
            Ok(w) => assert!(w.set(b"ROBIGO_FORK_TEST", b"1"), "the child's setenv"),
            Err(crate::clib::EnvWriteRefused::SeveralIsolates)
                if crate::clib::glibc_version() < crate::clib::MIN_GLIBC => {}
            Err(e) => panic!("the child's environment writer: {e:?}"),
        }
        let _ = cx.install(format!("robigo_fork_test_{}", std::process::id()).as_bytes()).unwrap();
    });
    if r.is_ok() { 0 } else { 1 }
}

/// Abort the whole test process if `done` is not set within `secs`: a fork
/// that never returns (a `prepare` handler stuck on a lock) hangs before any
/// deadline of the caller's could run.
pub(crate) fn watchdog(what: &'static str, secs: u64) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let d = std::sync::Arc::clone(&done);
    std::thread::spawn(move || {
        let until = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < until {
            if d.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        eprintln!("{what}: no progress in {secs} s (a fork or its handlers hung)");
        std::process::abort();
    });
    done
}

/// Fork while another thread holds a lock (`hold` returns its guard), and
/// check that the child finishes `child_work`.
///
/// The overlap is observed, not timed: the holder releases only once a fork
/// has entered the `prepare` handler (`robigo_heap::fork_prepares`). The
/// counter is bumped before `prepare` takes its locks. `prepare` takes
/// every one of them, so it must wait until the holder releases this one,
/// and the fork happens only once `prepare` holds it. The child therefore
/// starts with the lock owned by its own (forking) thread, which `child`
/// releases. Without the handlers no `prepare` runs: the holder gives up
/// after 2 s, by which time the fork has happened with the lock held, and
/// the child blocks for ever (the negative control: with registration
/// disabled, every one of these tests times out).
fn fork_while_held<G: 'static>(what: &'static str, hold: fn() -> G) {
    // serialized: each test forks while its own lock is held, and another
    // test's fork would enter `prepare` and release this one's holder
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    // load-bearing: starting an isolate publishes this crate's fork locks
    // (`with_options` repeats the constructor's `set_fork_locks`), so the
    // test means the same if a linker dropped the constructor
    drop(OwnedIsolate::new());
    let done = watchdog(what, 60);
    let (held_tx, held_rx) = mpsc::channel();
    let before = robigo_heap::fork_prepares();
    let holder = std::thread::spawn(move || {
        let g = hold();
        held_tx.send(()).unwrap();
        let give_up = Instant::now() + Duration::from_secs(2);
        while robigo_heap::fork_prepares() == before && Instant::now() < give_up {
            std::thread::sleep(Duration::from_millis(1));
        }
        drop(g);
    });
    held_rx.recv().unwrap();
    // SAFETY: the child runs `child_work` and exits without returning into
    // the test harness (`_exit`).
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "{what}: fork failed: {}", std::io::Error::last_os_error());
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
        assert!(r == 0, "{what}: waitpid failed: {}", std::io::Error::last_os_error());
        if Instant::now() > deadline {
            // SAFETY: our own child.
            unsafe { libc::kill(pid, libc::SIGKILL) };
            panic!("{what}: the child forked while another thread held it never finished");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    done.store(true, std::sync::atomic::Ordering::SeqCst);
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
    // SAFETY: the holder thread interns nothing while it holds the lock and
    // drops the guard on its own thread.
    fork_while_held("the arena", || unsafe { robigo_heap::hold_arena_lock_for_test() });
}

/// A process-global lazy initializer caught mid-way by a fork (Sol r4 S9):
/// one thread is inside `once_excluding_fork`'s initializer, paused, when
/// this one forks. The fork waits until the initializer is published (it
/// holds `FORK`), so the child finds the cell initialized instead of
/// "running" with no thread to finish it. A plain `get_or_init` here (the
/// negative control) leaves the child waiting for ever.
#[test]
fn a_fork_during_a_lazy_initializer_waits_for_it() {
    static CELL: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let done = watchdog("a lazy initializer", 60);
    let (inside_tx, inside_rx) = mpsc::channel();
    let initer = std::thread::spawn(move || {
        robigo_heap::once_excluding_fork(&CELL, || {
            inside_tx.send(()).unwrap();
            // long enough for the fork below to be attempted meanwhile
            std::thread::sleep(Duration::from_millis(300));
            7
        });
    });
    inside_rx.recv().unwrap();
    // SAFETY: the child reads the cell and exits without returning into the
    // test harness.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork failed: {}", std::io::Error::last_os_error());
    if pid == 0 {
        let v = *robigo_heap::once_excluding_fork(&CELL, || 0);
        // SAFETY: ends the child at once.
        unsafe { libc::_exit(if v == 7 { 0 } else { 1 }) };
    }
    initer.join().unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut status = 0;
    loop {
        // SAFETY: our own child; `status` is writable.
        let r = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        if r == pid {
            break;
        }
        assert!(r == 0, "waitpid failed: {}", std::io::Error::last_os_error());
        if Instant::now() > deadline {
            // SAFETY: our own child.
            unsafe { libc::kill(pid, libc::SIGKILL) };
            panic!("the child forked during a lazy initializer never finished");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0, "the child failed (status {status})");
}
