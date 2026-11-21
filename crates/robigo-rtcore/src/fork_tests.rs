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
/// locks. Exit status 0 if it all worked. Each step it reaches is written
/// to `steps` (a pipe's write end; `write(2)` takes no lock), so a parent
/// whose child hangs can say where.
fn child_work(steps: i32) -> i32 {
    let step = |b: u8| {
        // SAFETY: one byte from a live local to the pipe's write end.
        unsafe { libc::write(steps, (&b as *const u8).cast(), 1) };
    };
    let r = std::panic::catch_unwind(|| {
        // the two locks of this crate, taken directly: free in the child on
        // any glibc (the writer below refuses before it takes `ENV` when an
        // older glibc counts several isolates)
        step(b'L');
        drop(crate::isolate::live_isolates());
        step(b'E');
        drop(crate::clib::env_write_lock());
        step(b'I');
        let mut iso = OwnedIsolate::new();
        step(b'G');
        // SAFETY: the child has one thread, this one: no other reads the
        // environment while it writes.
        iso.grant_env_writes(unsafe { crate::clib::EnvWrites::assume() });
        let cx = iso.cx();
        // other tests' isolates still count in the child: on a glibc older
        // than MIN_GLIBC that is the writer's documented refusal
        step(b'W');
        match crate::clib::env_writer(cx) {
            Ok(w) => {
                step(b'S');
                assert!(w.set(b"ROBIGO_FORK_TEST", b"1"), "the child's setenv")
            }
            Err(crate::clib::EnvWriteRefused::SeveralIsolates)
                if crate::clib::glibc_version() < crate::clib::MIN_GLIBC => {}
            Err(e) => panic!("the child's environment writer: {e:?}"),
        }
        step(b'N');
        let _ = cx.install(format!("robigo_fork_test_{}", std::process::id()).as_bytes()).unwrap();
        step(b'D');
        drop(iso);
        step(b'X');
    });
    if r.is_ok() { 0 } else { 1 }
}

/// What a hung child is doing: the steps its pipe received, and the kernel's
/// view of it (`/proc/<pid>/syscall`: a futex wait names the lock's address;
/// `wchan`), for a timeout's message.
fn hung_child_report(pid: i32, steps_rd: i32) -> String {
    let mut buf = [0u8; 64];
    // SAFETY: a nonblocking read end, into a local buffer.
    let n = unsafe { libc::read(steps_rd, buf.as_mut_ptr().cast(), buf.len()) };
    let steps = if n > 0 { String::from_utf8_lossy(&buf[..n as usize]).into_owned() } else { String::new() };
    let proc = |f: &str| std::fs::read_to_string(format!("/proc/{pid}/{f}")).unwrap_or_else(|e| format!("({e})"));
    format!(
        "steps {steps:?} (L live, E env, I isolate, G grant, W writer, S setenv, N install, D done, X dropped); \
         syscall {:?}; wchan {:?}; locks: LIVE_ISOLATES {:p}, ENV {:p}",
        proc("syscall").trim(),
        proc("wchan"),
        crate::isolate::live_isolates_addr(),
        crate::clib::env_lock_addr(),
    )
}

/// Abort the whole test process if `done` is not set within `secs`: a fork
/// that never returns (a `prepare` handler stuck on a lock) hangs before any
/// deadline of the caller's could run.
/// The watchdog is disarmed when its guard drops, so a test that fails
/// (rather than hangs) ends with its own panic, not an abort later on.
pub(crate) fn watchdog(what: &'static str, secs: u64) -> Watchdog {
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let d = std::sync::Arc::clone(&done);
    std::thread::spawn(move || {
        let until = Instant::now() + Duration::from_secs(secs);
        // (the flag read after the last sleep too: a guard dropped during
        // it disarms)
        while !d.load(std::sync::atomic::Ordering::SeqCst) {
            if Instant::now() >= until {
                abort_saying(&format!("{what}: no progress in {secs} s (a fork or its handlers hung)\n"));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    Watchdog(done)
}

/// Abort the process, saying why on fd 2 only if that cannot block: raw
/// `write(2)`, not `eprintln!` (std's stderr lock may be the stuck one), and
/// only when `poll` finds room (a full stderr pipe may be what stalled the
/// fork). A message shorter than `PIPE_BUF` then goes in one write.
pub(crate) fn abort_saying(msg: &str) -> ! {
    let msg = msg.to_owned();
    // the message from a thread of its own, the abort after a short wait
    // whether or not the write finished (a full stderr pipe can block it
    // even after `poll` said there was room; Sol S4)
    std::thread::spawn(move || {
        // SAFETY: a live buffer to fd 2.
        unsafe { libc::write(2, msg.as_ptr().cast(), msg.len()) };
    });
    std::thread::sleep(Duration::from_millis(200));
    std::process::abort();
}

/// [`watchdog`]'s guard: dropping it disarms the watchdog.
pub(crate) struct Watchdog(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Wait for a killed child to end (`waitpid`, again after a signal).
pub(crate) fn reap(pid: i32) {
    let mut status = 0;
    // SAFETY: our own child; `status` is writable.
    while unsafe { libc::waitpid(pid, &mut status, 0) } == -1
        && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR)
    {}
}

/// The fork tests, one at a time: each forks while its own lock is held,
/// and another test's fork would enter `prepare` and release this one's
/// holder before this test's fork does.
fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    ONE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Wait up to 20 s for the child `pid` (which reports its steps to the
/// pipe `fds`), close the pipe, and give its status. A child that never
/// finishes is killed and reaped, and the test fails, saying where it hung.
fn wait_for_child(what: &str, pid: i32, fds: [i32; 2]) -> i32 {
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
            let report = hung_child_report(pid, fds[0]);
            // SAFETY: our own child, killed and reaped; the pipe's ends, ours.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
                reap(pid);
                libc::close(fds[0]);
                libc::close(fds[1]);
            }
            panic!("{what}: the child never finished: {report}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // SAFETY: the pipe's two ends, ours.
    unsafe {
        libc::close(fds[0]);
        libc::close(fds[1]);
    }
    status
}

/// A pipe for a child's step reports (`child_work`).
fn step_pipe(what: &str) -> [i32; 2] {
    let mut fds = [0i32; 2];
    // SAFETY: a two-int array for the pipe's ends.
    assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) }, 0, "{what}: pipe");
    fds
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
    let _one = one_at_a_time();
    // load-bearing: starting an isolate publishes this crate's fork locks
    // (`with_options` repeats the constructor's `set_fork_locks`), so the
    // test means the same if a linker dropped the constructor
    drop(OwnedIsolate::new());
    let _dog = watchdog(what, 60);
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
    let fds = step_pipe(what);
    // SAFETY: the child runs `child_work` and exits without returning into
    // the test harness (`_exit`).
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "{what}: fork failed: {}", std::io::Error::last_os_error());
    if pid == 0 {
        let s = child_work(fds[1]);
        // SAFETY: ends the child at once, running no handler of the parent's.
        unsafe { libc::_exit(s) };
    }
    holder.join().unwrap();
    let status = wait_for_child(what, pid, fds);
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

/// A fork that fails still runs the `parent` handler, which releases every
/// lock `prepare` took (glibc: `fork` returning EAGAIN runs prepare, then
/// parent). A process limited to no processes (RLIMIT_NPROC 0; a root
/// process drops to `nobody` first, root being exempt) fails a fork and
/// then must still start an isolate, write the environment and intern.
#[test]
fn a_failed_fork_releases_the_locks() {
    let _one = one_at_a_time();
    drop(OwnedIsolate::new());
    let _dog = watchdog("a failed fork", 60);
    let fds = step_pipe("a failed fork");
    // SAFETY: the child (one thread) runs the code below and exits without
    // returning into the test harness.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "a failed fork: the outer fork failed: {}", std::io::Error::last_os_error());
    if pid == 0 {
        // SAFETY: plain system calls in a one-thread child; `_exit` ends it.
        unsafe {
            if libc::geteuid() == 0 && (libc::setgid(65534) != 0 || libc::setuid(65534) != 0) {
                libc::_exit(3);
            }
            let none = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
            // the handlers must have run around the failed fork: `prepare`
            // took every lock and `parent` released it
            let before = robigo_heap::fork_prepares();
            if libc::setrlimit(libc::RLIMIT_NPROC, &none) != 0 {
                libc::_exit(4);
            }
            match libc::fork() {
                // (the fork was to fail: a grandchild ends at once)
                0 => libc::_exit(5),
                p if p > 0 => libc::_exit(6),
                _ => {}
            }
            if std::io::Error::last_os_error().raw_os_error() != Some(libc::EAGAIN) {
                libc::_exit(7);
            }
            if robigo_heap::fork_prepares() != before + 1 {
                libc::_exit(8);
            }
        }
        let s = child_work(fds[1]);
        // SAFETY: ends the child at once.
        unsafe { libc::_exit(s) };
    }
    let status = wait_for_child("a failed fork", pid, fds);
    // 3, 4: the child could not drop its rights or set the limit; 5, 6: the
    // fork it meant to fail succeeded; 7: it failed for another reason; 8:
    // no `prepare` ran around it
    assert!(
        libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
        "a failed fork: the child failed (status {status}, exit {})",
        libc::WEXITSTATUS(status)
    );
}

/// std's stdout lock, held by a thread that writes (another isolate's
/// console, libtest's result lines): the child's isolate teardown flushes
/// its console, which takes it (#894's `rt` failure on CI).
#[test]
fn a_child_forked_while_stdout_is_locked_tears_its_isolate_down() {
    // the holder takes no robigo lock while it holds it
    #[allow(clippy::disallowed_methods)]
    fork_while_held("stdout", || std::io::stdout().lock());
}

/// std's stderr lock, held by a thread that writes a warning or a panic's
/// message.
#[test]
fn a_child_forked_while_stderr_is_locked_tears_its_isolate_down() {
    // the holder takes no robigo lock while it holds it
    #[allow(clippy::disallowed_methods)]
    fork_while_held("stderr", || std::io::stderr().lock());
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
    let _one = one_at_a_time();
    let _dog = watchdog("a lazy initializer", 60);
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
            // SAFETY: our own child, killed and reaped.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
                reap(pid);
            }
            panic!("the child forked during a lazy initializer never finished");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0, "the child failed (status {status})");
}
