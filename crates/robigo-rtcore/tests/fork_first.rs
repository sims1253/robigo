//! The fork handlers exist before the first heap does (robigo-heap's
//! `fork.rs` registers them before `main`): a fork made before any isolate
//! runs them. Its own test binary, so that no other test makes a heap
//! first.

#[test]
#[cfg(all(target_os = "linux", not(miri)))]
fn the_fork_handlers_run_before_the_first_heap_exists() {
    let before = robigo_heap::fork_prepares();
    // SAFETY: the child exits at once.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork");
    if pid == 0 {
        // SAFETY: ends the child, running no handler of the parent's.
        unsafe { libc::_exit(0) };
    }
    let mut status = 0;
    // SAFETY: our own child; `status` is writable.
    assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
    assert!(robigo_heap::fork_prepares() > before, "prepare ran for a fork before the first heap");
}
