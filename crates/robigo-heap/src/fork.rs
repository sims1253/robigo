//! The process's `fork` handlers: one `pthread_atfork` triple, so that a
//! child forked while another thread holds one of the process's locks does
//! not inherit it held by a thread the child does not have.
//!
//! **Registration.** On Linux an `.init_array` constructor registers the
//! handlers before `main`, while the process has one thread. Linux with
//! glibc is the only target robigo-rtcore builds for (`clib.rs`'s
//! `compile_error!`). Elsewhere (robigo-heap alone, on another unix) the
//! first `Core::new` registers them. There a fork racing that first
//! registration on another thread is not covered: a second thread's heap
//! may take the arena's lock before the first thread's `pthread_atfork`
//! returns.
//!
//! **The process's locks and their order**, the one place it is written:
//! 0. `FORK` (here): held by a fork's handlers from `prepare` to `parent` /
//!    `child`, so two forks never interleave their handlers;
//! 1. `LIVE_ISOLATES` (robigo-rtcore `isolate.rs`): the live-isolate count,
//!    held while an isolate starts or ends and for a whole `Sys.setenv` /
//!    `Sys.unsetenv`;
//! 2. `ENV` (robigo-rtcore `clib.rs`): the process environment's lock,
//!    shared for a C call that reads the environment (`dyn.load`'s
//!    `dlopen` and `dyn.unload`'s `dlclose` included, across the
//!    library's initializers and finalizers: `clib::with_env_shared`),
//!    exclusive for a write. glibc's loader locks come after it, since
//!    those calls take them with `ENV` held: code run under a loader lock
//!    (an initializer or finalizer, a `dl_iterate_phdr` callback, a TLS
//!    initializer) takes no `ENV` and does not fork, or it waits for `ENV`
//!    while the `dlopen` holding `ENV` waits for the loader lock. glibc's
//!    own calls keep the order (glibc 2.41's and master's sources): `fork`
//!    takes no loader lock (posix/fork.c: the atfork `prepare` handlers,
//!    then NSS's, stdio's and malloc's locks; the child re-initializes
//!    `dl_load_lock`), `posix_spawn` and `system` clone with `CLONE_VFORK`
//!    without either, and `__cxa_thread_atexit_impl` takes the loader lock
//!    but never `ENV`;
//! 3. the symbol arena's lock (`arena.rs`);
//! 4. std's stdout lock, then std's stderr lock: held by a thread while it
//!    writes (an isolate's console, a warning, a panic's message). Without
//!    them a child forked while another thread writes blocks on its own
//!    first write, or on its isolate's teardown, which flushes the console.
//!    (glibc's `fork` makes the C library's own stdio locks usable in the
//!    child by resetting them there; std's cannot be reset from outside.)
//!
//! A thread takes them in this order and never the other way: in
//! particular, it takes none of 0–3 while it holds std's stdout or stderr
//! lock (a `stdout().lock()` held across a robigo call breaks that; clippy's
//! `disallowed-methods`, in the workspace's `clippy.toml`, refuses std's
//! `Stdout::lock` and `Stderr::lock` except at sites allowed with a reason).
//! `prepare` takes 0, 1, 2 (exclusive), 3 and 4, in that order. `parent` and `child`
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
//! **std's stdin lock** is not one of them: a thread reading the console
//! holds it for as long as the read blocks, and a fork would wait for the
//! input. A child forked while another thread reads std's stdin must not
//! read it.
//!
//! **Lazy process-global state.** A `OnceLock` caught mid-initialization by
//! a fork would stay "running" in the child. So the process-global
//! initializers an isolate's start, an environment write or an install
//! reaches run under `FORK` ([`once_excluding_fork`]):
//! - robigo-heap: `heapsize::settings`, `shared::shared`,
//!   `set_symbol_bits`, `gc.time`'s start instant;
//! - robigo-rtcore: `clib`'s two C locales and the glibc version.
//!
//! **What `prepare` waits for.** It blocks until every holder releases.
//! - A reader holding `ENV` shared for a C call (`localtime`, the locale
//!   calls, a `dlopen` with its library's initializers) finishes that call
//!   first. An initializer that forks on that same thread would wait on
//!   its own hold, which deadlocks, as a fork from a thread holding a
//!   loader lock that the `dlopen` waits for would (`clib::with_env_shared`
//!   says why nothing of robigo's in it can take `FORK` or
//!   `LIVE_ISOLATES`: every C API export refuses a call made outside a
//!   native entry).
//! - A thread writing to std's stdout or stderr finishes that write first.
//!   A write blocked on a full pipe holds the fork until the pipe's reader
//!   drains it, so the reader must not be waiting on this fork.
//! - A thread blocked waiting for one of these locks across the fork stays
//!   blocked in the parent until `parent` releases it, and does not exist
//!   in the child.
//! - No robigo code forks while it holds any of these locks: the forking
//!   thread would wait on itself.

#![allow(unsafe_code)]
// (Miri runs no fork: the handlers and their state are compiled, unused)
#![cfg_attr(miri, allow(dead_code))]

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

// The callbacks travel as data pointers: a `fn()` is one on every target
// this crate builds for, and the conversion back is a transmute.
const _: () = assert!(std::mem::size_of::<fn()>() == std::mem::size_of::<*mut ()>());

/// Give the fork handlers the embedder's locks, taken before the arena's
/// in `prepare` and released after it. The embedder calls it before any of
/// its locks can be held: robigo-rtcore from a constructor of its own
/// (before `main`), and again before an isolate's first lock.
///
/// What the handlers need of the two functions:
/// - `take` takes the embedder's locks in the process's order (the module
///   doc), all of them before the arena's;
/// - `release` releases exactly what `take` took, on the same thread;
/// - neither allocates from a robigo heap, forks, or panics.
///
/// The stores are made under `FORK`, so no fork is between `prepare` and
/// `parent`/`child` while the pair changes: each fork releases with the
/// pair it took with.
pub fn set_fork_locks(take: fn(), release: fn()) {
    // SAFETY: the static mutex, initialized; the stores below need no other
    // lock, and nothing here can unwind while it is held.
    unsafe { libc::pthread_mutex_lock(FORK.0.get()) };
    EMBEDDER_RELEASE.store(release as *const () as *mut (), Ordering::SeqCst);
    EMBEDDER_TAKE.store(take as *const () as *mut (), Ordering::SeqCst);
    // SAFETY: taken above by this thread.
    unsafe { libc::pthread_mutex_unlock(FORK.0.get()) };
}

/// The thread that holds `FORK` for [`once_excluding_fork`] (its
/// `pthread_self`), 0 when none: one initializer that reaches another must
/// not take it twice. An atomic, not a thread-local: L6 allows the process
/// one thread-local, the isolate's. Only the holder ever reads its own id
/// here: it stores it after taking `FORK` and clears it before releasing.
static FORK_HOLDER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// This thread's id for [`FORK_HOLDER`] (never 0 on glibc: a thread
/// descriptor's address).
fn this_thread() -> usize {
    // SAFETY: no arguments; returns the calling thread's id.
    unsafe { libc::pthread_self() as usize }
}

/// `cell.get_or_init(f)` with no fork between `f`'s start and the value's
/// publication. A process-global `OnceLock` caught by a fork mid-initialization
/// stays "running" in the child, where no thread will finish it, and the
/// child's first use of it then waits for ever (Sol r4 S9).
/// - Holding `FORK` for the initialization keeps every fork out until it
///   is published.
/// - A fork already between its handlers holds `FORK`, so the initialization
///   waits until that fork is over.
/// - The fast path, an initialized cell, takes no lock.
/// - An initializer that reaches another one takes `FORK` once (it is not
///   recursive; [`FORK_HOLDER`] says whether this thread holds it).
///
/// `f` must not fork, and must take only the process's later locks
/// (`LIVE_ISOLATES`, `ENV`, the arena): `FORK` comes first in the order.
/// For the same reason, a cell that may not be initialized yet must not be
/// read while the caller holds one of those later locks: read it first.
/// `clib::env_writer` reads the glibc version before `LIVE_ISOLATES`, and
/// `arena::install` makes the shared objects before the arena's lock.
/// Every process-global lazy initializer that an isolate's start, an
/// environment write or an install reaches goes through this.
pub fn once_excluding_fork<T: Send + Sync>(cell: &'static std::sync::OnceLock<T>, f: impl FnOnce() -> T) -> &'static T {
    if let Some(v) = cell.get() {
        return v;
    }
    if FORK_HOLDER.load(Ordering::SeqCst) == this_thread() {
        return cell.get_or_init(f);
    }
    struct Held;
    impl Drop for Held {
        fn drop(&mut self) {
            FORK_HOLDER.store(0, Ordering::SeqCst);
            // SAFETY: taken below by this thread.
            unsafe { libc::pthread_mutex_unlock(FORK.0.get()) };
        }
    }
    // SAFETY: the static mutex, initialized; released by `Held`'s drop, on
    // this thread, also if `f` unwinds.
    unsafe { libc::pthread_mutex_lock(FORK.0.get()) };
    FORK_HOLDER.store(this_thread(), Ordering::SeqCst);
    let _held = Held;
    cell.get_or_init(f)
}

/// `FORK` held by the caller until the guard drops, as a fork's `prepare`
/// holds it: a test's stand-in for a fork between its handlers. The guard is
/// neither `Send` nor `Sync`.
///
/// # Safety
/// The caller forks nothing, and starts no lazy initialization of its own,
/// while it holds the guard; it drops it on this thread.
#[cfg(feature = "test-support")]
pub unsafe fn hold_fork_lock_for_test() -> impl Drop {
    struct Held(std::marker::PhantomData<*const ()>);
    impl Drop for Held {
        fn drop(&mut self) {
            // SAFETY: taken below by this thread.
            unsafe { libc::pthread_mutex_unlock(FORK.0.get()) };
        }
    }
    // SAFETY: the static mutex, initialized.
    unsafe { libc::pthread_mutex_lock(FORK.0.get()) };
    Held(std::marker::PhantomData)
}

/// How many forks have entered the `prepare` handler (a test's
/// handshake).
#[cfg(feature = "test-support")]
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
        // SAFETY: stored by `set_fork_locks` from a `fn()` of the same size
        // (the assertion above).
        let take: fn() = unsafe { std::mem::transmute::<*mut (), fn()>(take) };
        take();
        TOOK_EMBEDDER.store(true, Ordering::Relaxed);
    }
    crate::arena::lock_for_fork();
    // the order's last two: every robigo lock is already held
    #[allow(clippy::disallowed_methods)]
    let std_io = (std::io::stdout().lock(), std::io::stderr().lock());
    // SAFETY: the stash is this fork's alone: `FORK`, held, serializes the
    // forks' handlers, and `release` empties it.
    unsafe { *STD_IO.0.get() = Some(std_io) };
}

/// std's stdout and stderr locks, taken by `prepare` and dropped by
/// `release` on the same thread (the forking thread, or its copy in the
/// child: std's lock is reentrant per thread, and the child's thread keeps
/// the forking thread's identity).
struct StdIo(UnsafeCell<Option<(std::io::StdoutLock<'static>, std::io::StderrLock<'static>)>>);

// SAFETY: read and written only by a fork's handlers, under `FORK`, on the
// forking thread.
unsafe impl Sync for StdIo {}

static STD_IO: StdIo = StdIo(UnsafeCell::new(None));

extern "C" fn release() {
    // SAFETY: as in `prepare`: under `FORK`, which this fork's handlers hold.
    if let Some((out, err)) = unsafe { (*STD_IO.0.get()).take() } {
        // the reverse of `prepare`'s order
        drop(err);
        drop(out);
    }
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
        if r != 0 {
            // not registered: a later caller (the next `Core::new`) tries
            // again
            REGISTERED.store(false, Ordering::SeqCst);
            panic!("robigo: pthread_atfork failed ({r})");
        }
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
