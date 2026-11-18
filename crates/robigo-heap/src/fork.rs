//! The process's `fork` handlers: one `pthread_atfork` triple, registered
//! before `main` (an `.init_array` constructor, while the process has one
//! thread), so that a child forked while another thread holds one of the
//! process's locks does not inherit it held by a thread the child does not
//! have.
//!
//! **The process's locks and their order**, the one place it is written:
//! 0. `FORK` (here): held by a fork's handlers from `prepare` to `parent` /
//!    `child`, so two forks never interleave their handlers;
//! 1. `LIVE_ISOLATES` (robigo-rtcore `isolate.rs`): the live-isolate count,
//!    held while an isolate starts or ends and for a whole `Sys.setenv` /
//!    `Sys.unsetenv`;
//! 2. `ENV` (robigo-rtcore `clib.rs`): the process environment's lock,
//!    shared for a C call that reads the environment, exclusive for a
//!    write;
//! 3. the symbol arena's lock (`arena.rs`).
//!
//! A thread takes them in this order and never the other way. `prepare`
//! takes 0, 1, 2 (exclusive) and 3, in that order. `parent` and `child`
//! release them in the reverse order, and release only what that `prepare`
//! took. robigo-rtcore, which owns 1 and 2, gives their take and release to
//! [`set_fork_locks`]. A heap used without it (robigo-heap's own tests) has
//! only 0 and 3.
//!
//! **std's environment lock** is not one of them.
//! - robigo never takes it for writing: the environment writer calls the C
//!   library's `setenv`/`unsetenv` under `ENV` exclusive.
//! - std's readers (`std::env::var`) take it shared, and a fork can land
//!   while one does. The child then inherits a reader count that no thread
//!   of its own will drop. That blocks only std's writers, and robigo has
//!   none, so the child's `Sys.setenv` still writes.
//! - `std::process::Command`'s fork fallback holds it shared across `fork`.
//!   robigo-rt does not spawn through it, and when an embedder does,
//!   `prepare` waits for nothing that waits on std's lock.
//!
//! **What `prepare` waits for.** It blocks until every holder releases.
//! - A reader holding `ENV` shared for a C call (`localtime`, the locale
//!   calls) finishes that call first.
//! - A thread blocked waiting for one of these locks across the fork stays
//!   blocked in the parent until `parent` releases it, and does not exist
//!   in the child.
//! - No robigo code forks while it holds any of these locks: the forking
//!   thread would wait on itself.

#![allow(unsafe_code)]

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};

/// The embedder's locks: take them, release them (functions of no
/// arguments), in the order above; null until [`set_fork_locks`]. Plain
/// atomics, with no "being set" state a fork could catch: a fork sees
/// both, or neither (`release` is stored first, `take` last).
static EMBEDDER_TAKE: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());
static EMBEDDER_RELEASE: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());

/// Whether the current fork's `prepare` took the embedder's locks: `parent`
/// and `child` release only what it took. `FORK` serializes forks, so one
/// fork's handlers alone read and write it.
static TOOK_EMBEDDER: AtomicBool = AtomicBool::new(false);

/// Lock 0: one fork's handlers at a time.
struct ForkLock(UnsafeCell<libc::pthread_mutex_t>);

// SAFETY: a pthread mutex is made to be shared between threads; it is used
// only through the pthread calls below.
unsafe impl Sync for ForkLock {}

static FORK: ForkLock = ForkLock(UnsafeCell::new(libc::PTHREAD_MUTEX_INITIALIZER));

/// How many forks have entered `prepare`: a test's evidence that a fork
/// reached the handlers while it held a lock.
static PREPARES: AtomicU64 = AtomicU64::new(0);

static REGISTERED: AtomicBool = AtomicBool::new(false);

/// Give the fork handlers the embedder's locks, taken before the arena's
/// in `prepare` and released after it. The embedder calls it before any of
/// its locks can be held: robigo-rtcore from a constructor of its own
/// (before `main`), and again before an isolate's first lock (a later call
/// with the same functions changes nothing). A fork in progress when it is
/// called releases what its own `prepare` took.
pub fn set_fork_locks(take: fn(), release: fn()) {
    EMBEDDER_RELEASE.store(release as *mut (), Ordering::SeqCst);
    EMBEDDER_TAKE.store(take as *mut (), Ordering::SeqCst);
}

/// How many forks have entered the `prepare` handler (a test's
/// handshake).
#[doc(hidden)]
pub fn fork_prepares() -> u64 {
    PREPARES.load(Ordering::SeqCst)
}

extern "C" fn prepare() {
    // SAFETY: the static mutex, initialized; released by `release` in the
    // parent, or in the child by the same (forking) thread's copy.
    unsafe { libc::pthread_mutex_lock(FORK.0.get()) };
    PREPARES.fetch_add(1, Ordering::SeqCst);
    let take = EMBEDDER_TAKE.load(Ordering::SeqCst);
    if !take.is_null() {
        // SAFETY: stored by `set_fork_locks` from a `fn()`.
        let take: fn() = unsafe { std::mem::transmute::<*mut (), fn()>(take) };
        take();
        TOOK_EMBEDDER.store(true, Ordering::Relaxed);
    }
    crate::arena::lock_for_fork();
}

extern "C" fn release() {
    crate::arena::unlock_after_fork();
    if TOOK_EMBEDDER.swap(false, Ordering::Relaxed) {
        // `release` was stored before `take`, which this fork's `prepare`
        // saw
        let release = EMBEDDER_RELEASE.load(Ordering::SeqCst);
        // SAFETY: stored by `set_fork_locks` from a `fn()`, and not null.
        let release: fn() = unsafe { std::mem::transmute::<*mut (), fn()>(release) };
        release();
    }
    // SAFETY: the mutex this fork's `prepare` took (on this thread, or its
    // copy in the child).
    unsafe { libc::pthread_mutex_unlock(FORK.0.get()) };
}

/// Register the handlers, once per process. The constructor below does it
/// before `main`; `Core::new` calls it too, in case a linker dropped the
/// constructor. It is a swap, not a `Once`, so a fork during it leaves the
/// child no initialization in progress to wait on.
pub(crate) fn register() {
    // (Miri runs no fork)
    #[cfg(not(miri))]
    if !REGISTERED.swap(true, Ordering::SeqCst) {
        // SAFETY: three `extern "C"` functions of no arguments, as
        // pthread_atfork takes them; `release` serves parent and child.
        let r = unsafe { libc::pthread_atfork(Some(prepare), Some(release), Some(release)) };
        assert_eq!(r, 0, "robigo: pthread_atfork");
    }
}

/// The constructor: [`register`] before `main`, while the process has one
/// thread, so no fork can race the registration.
#[cfg(all(target_os = "linux", not(miri)))]
extern "C" fn register_at_load() {
    register();
}

// SAFETY: an `.init_array` entry is a function of no arguments that the
// loader calls once, before `main`. `register_at_load` calls only
// `pthread_atfork` and an atomic swap, which need nothing of std's
// runtime.
#[cfg(all(target_os = "linux", not(miri)))]
#[used]
#[unsafe(link_section = ".init_array")]
static REGISTER_AT_LOAD: extern "C" fn() = register_at_load;
