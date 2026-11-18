//! The process's `fork` handlers: one `pthread_atfork` triple, registered
//! once and eagerly (the first `Core::new`), so that a child forked while
//! another thread holds one of the process's locks does not inherit it held
//! by a thread the child does not have.
//!
//! **The process's locks and their order**, the one place it is written:
//! 1. `LIVE_ISOLATES` (robigo-rtcore `isolate.rs`): the live-isolate count,
//!    held while an isolate starts or ends and for a whole `Sys.setenv` /
//!    `Sys.unsetenv`;
//! 2. `ENV` (robigo-rtcore `clib.rs`): the process environment's lock,
//!    shared for a C call that reads the environment, exclusive for a
//!    write;
//! 3. std's environment lock (inside `std::env::set_var`), which robigo
//!    takes only under `ENV` exclusive;
//! 4. the symbol arena's lock (`arena.rs`).
//!
//! A thread takes them in this order and never the other way. `prepare`
//! takes 1, 2 (exclusive) and 4, in that order. Lock 3 is not taken:
//! robigo calls `set_var` only under `ENV` exclusive, so no robigo thread
//! is inside it once `prepare` holds `ENV`. `parent` and `child` release
//! them in the reverse order. robigo-rtcore, which owns 1 and 2, gives
//! their prepare and release to [`set_fork_locks`] before its first heap
//! exists. A heap used without it (robigo-heap's own tests) has only the
//! arena's lock.
//!
//! **What `prepare` waits for.** It blocks until every holder releases.
//! - A thread blocked waiting for one of these locks across the fork stays
//!   blocked in the parent until `parent` releases it, and does not exist
//!   in the child.
//! - A reader holding `ENV` shared for a C call (`localtime`, the locale
//!   calls) finishes that call first.
//! - No robigo code forks while it holds any of these locks: the forking
//!   thread would wait on itself.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// The embedder's locks: (take them, release them), in the order above.
static EMBEDDER: OnceLock<(fn(), fn())> = OnceLock::new();

/// Whether the current fork's `prepare` took the embedder's locks: `parent`
/// and `child` release only what it took (set and read by the forking
/// thread, between the two calls).
static TOOK_EMBEDDER: AtomicBool = AtomicBool::new(false);

static REGISTERED: std::sync::Once = std::sync::Once::new();

/// Give the fork handlers the embedder's locks, taken before the arena's
/// in `prepare` and released after it. The first call stays.
pub fn set_fork_locks(take: fn(), release: fn()) {
    let _ = EMBEDDER.set((take, release));
}

extern "C" fn prepare() {
    if let Some(&(take, _)) = EMBEDDER.get() {
        take();
        TOOK_EMBEDDER.store(true, Ordering::Relaxed);
    }
    crate::arena::lock_for_fork();
}

extern "C" fn release() {
    crate::arena::unlock_after_fork();
    if TOOK_EMBEDDER.swap(false, Ordering::Relaxed)
        && let Some(&(_, release)) = EMBEDDER.get()
    {
        release();
    }
}

/// Register the handlers, once per process (the first `Core::new`).
pub(crate) fn register() {
    // (Miri runs no fork)
    #[cfg(not(miri))]
    REGISTERED.call_once(|| {
        // SAFETY: three `extern "C"` functions of no arguments, as
        // pthread_atfork takes them; `release` serves parent and child.
        let r = unsafe { libc::pthread_atfork(Some(prepare), Some(release), Some(release)) };
        assert_eq!(r, 0, "robigo: pthread_atfork");
    });
    #[cfg(miri)]
    let _ = &REGISTERED;
}
