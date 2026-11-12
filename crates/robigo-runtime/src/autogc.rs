//! Automatic collection of the runtime's reference cycles (#302).
//!
//! Values are reference counted, so what a cycle holds (an environment and
//! the closures or promises in it that point back) is freed only by a
//! collection that traces it: `gc()`'s cycle steps (the native bridge's, or
//! [`crate::reachability::reclaim_cycles`] without one). GNU R collects when
//! its heap is full, so a program never has to call `gc()` to get its
//! memory back; here a collection is due when what the interpreter's thread
//! holds reaches a threshold sized as GNU R sizes its vector heap
//! (memory.c's `R_VSize` and `AdjustHeapSize`).
//!
//! The decision depends only on what the program allocates and frees,
//! never on the allocator's page and commit state, the binary's layout, the
//! environment or other threads. The executable's allocator reports every
//! allocation and free of the interpreter's thread with its size
//! ([`allocated`], [`freed`]) to a counter that thread owns: the room left
//! under the threshold. A safe point that finds it spent collects. Other
//! threads count into their own counters, which nothing reads. A block one
//! thread allocates and another frees would stay counted on the first:
//! the interpreter's thread starts helper threads only through
//! [`crate::builtins::sysutils::OutputReader`], which leaves its count as
//! it was across the start, the join and the hand-over of the result.
//!
//! Without a counting allocator (the runtime's unit tests) nothing is due.
//!
//! A collection runs only where GNU R runs pending finalizers, at its
//! interrupt-check points: closure calls (the bytecode's
//! `START_BCFRAME_CALL`) and loop back edges (`GOTO`, `BRIFNOT`, `STEPFOR`;
//! each iteration of an AST loop), eval.c's `bc_check_sigint`. Arbitrary
//! code can run there in GNU R, but never at a native call's exit. So the
//! heap is checked against the threshold there, as GNU R's `allocVector3`
//! checks it where it can collect (`VHEAP_FREE() < alloc_size`). Garbage
//! that is not part of a cycle is freed the moment it is dropped. A peak of
//! temporaries that is freed before the next safe point therefore leaves
//! nothing over the threshold there, and nothing a collection could free.

use std::cell::Cell;

use crate::interp::Interp;

/// The room while no collection can be due (a thread's start, or before
/// [`enable`]). No program holds that much.
const OFF: isize = 1 << 62;

thread_local! {
    /// The held bytes at which a collection is due (`R_VSize` above
    /// `BOOT`). What the thread holds is `LIMIT - ROOM`, so a thread's
    /// counter starts at nothing held.
    static LIMIT: Cell<isize> = const { Cell::new(OFF) };
    /// The bytes held once the session was up (base and the default
    /// packages loaded). GNU R's sizes apply to what the program allocates
    /// beyond it: GNU R's own base takes a few megabytes of its heap,
    /// robigo's environments and compiled code tens.
    static BOOT: Cell<isize> = const { Cell::new(0) };
}

/// `ROOM`: the bytes this thread can hold before a collection is due,
/// `LIMIT` less what it holds. Negative: collect at the next safe point.
///
/// Every allocation and free updates it, so on x86-64 ELF it is a
/// thread-local of the executable's own TLS block, which an instruction
/// reaches at a fixed offset from `%fs` (the local-exec model). A
/// `thread_local!` in this crate takes two instructions: the offset
/// loaded from the GOT (relaxed to an immediate by the linker), then the
/// update, and rustc has no stable way to ask for local-exec. robigo is
/// only ever linked into executables, where local-exec is valid.
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
#[allow(unsafe_code)]
mod room {
    std::arch::global_asm!(
        ".pushsection .tdata,\"awT\",@progbits",
        ".balign 8",
        ".globl robigo_autogc_room",
        ".type robigo_autogc_room,@object",
        ".size robigo_autogc_room,8",
        "robigo_autogc_room:",
        ".quad {off}",
        ".popsection",
        off = const super::OFF,
    );

    #[inline(always)]
    pub fn sub(bytes: isize) {
        // SAFETY: an update of this thread's own counter (each thread has
        // its copy of the TLS block); nothing else is touched.
        unsafe {
            std::arch::asm!("subq {0}, %fs:robigo_autogc_room@tpoff", in(reg) bytes, options(att_syntax, nostack))
        }
    }

    #[inline(always)]
    pub fn add(bytes: isize) {
        // SAFETY: as in `sub`.
        unsafe {
            std::arch::asm!("addq {0}, %fs:robigo_autogc_room@tpoff", in(reg) bytes, options(att_syntax, nostack))
        }
    }

    #[inline(always)]
    pub fn get() -> isize {
        let room: isize;
        // SAFETY: a read of this thread's own counter.
        unsafe {
            std::arch::asm!(
                "movq %fs:robigo_autogc_room@tpoff, {0}",
                out(reg) room,
                options(att_syntax, nostack, readonly, preserves_flags)
            )
        };
        room
    }

    pub fn set(room: isize) {
        // SAFETY: a write of this thread's own counter.
        unsafe {
            std::arch::asm!("movq {0}, %fs:robigo_autogc_room@tpoff", in(reg) room, options(att_syntax, nostack, preserves_flags))
        }
    }
}

#[cfg(not(all(target_arch = "x86_64", target_os = "linux")))]
mod room {
    use std::cell::Cell;

    thread_local! {
        static ROOM: Cell<isize> = const { Cell::new(super::OFF) };
    }

    #[inline(always)]
    pub fn sub(bytes: isize) {
        ROOM.with(|r| r.set(r.get().wrapping_sub(bytes)));
    }

    #[inline(always)]
    pub fn add(bytes: isize) {
        ROOM.with(|r| r.set(r.get().wrapping_add(bytes)));
    }

    #[inline(always)]
    pub fn get() -> isize {
        ROOM.with(|r| r.get())
    }

    pub fn set(room: isize) {
        ROOM.with(|r| r.set(room));
    }
}

/// `sizeof(VECREC)`: GNU R's vector heap is counted in these.
const VECREC: f64 = 8.0;

// R: Defn.h:R_VSIZE; memory.c's growth settings (the defaults of
// init_gc_grow_settings)
const R_VSIZE: f64 = 67_108_864.0;
const MIN_FREE_FRAC: f64 = 0.2;
const V_GROW_FRAC: f64 = 0.70;
const V_SHRINK_FRAC: f64 = 0.30;
const V_GROW_INCR_FRAC: f64 = 0.2;
const V_GROW_INCR_MIN: f64 = 80_000.0 * VECREC;
const V_SHRINK_INCR_FRAC: f64 = 0.2;
const V_SHRINK_INCR_MIN: f64 = 0.0;

thread_local! {
    /// `R_VSize` in bytes, as the last adjustment left it.
    static V_SIZE: Cell<f64> = const { Cell::new(R_VSIZE) };
    /// A collection is running (its finalizers can reach safe points).
    static COLLECTING: Cell<bool> = const { Cell::new(false) };
    /// Collect at every `n`th safe point (`ROBIGO_CYCLES_TORTURE=n`; 0:
    /// off), and how many have passed.
    static TORTURE: Cell<u64> = const { Cell::new(0) };
    static TORTURE_SEEN: Cell<u64> = const { Cell::new(0) };
}

/// The allocator hands this thread `bytes` (the requested size). It is
/// called from the global allocator, so it must not allocate.
#[inline(always)]
pub fn allocated(bytes: usize) {
    room::sub(bytes as isize);
}

/// This thread frees a block of `bytes` (the size it was allocated with).
#[inline(always)]
pub fn freed(bytes: usize) {
    room::add(bytes as isize);
}

/// This thread takes over a block of `bytes` that another thread allocated
/// and that this one will free (`system()`'s output buffer), so that the
/// free is balanced.
pub(crate) fn adopt(bytes: usize) {
    allocated(bytes);
}

/// Runs `f`, then puts this thread's count back where it was: for std's
/// bookkeeping of a helper thread (the boxed closure, the result slot, the
/// handles), which a spawn allocates here and the thread that lets go last
/// frees, so a join or a detach frees some of it here or none. Only std's
/// spawn, join and drop of a handle go in `f`: what it allocates for the
/// program would go uncounted.
pub(crate) fn uncounted<R>(f: impl FnOnce() -> R) -> R {
    let room = room::get();
    let r = f();
    room::set(room);
    r
}

/// Is a collection due (the check at every safe point)?
#[inline(always)]
pub fn due() -> bool {
    room::get() < 0
}

/// The bytes this thread holds, as counted (net of what it freed).
pub fn held() -> isize {
    LIMIT.with(|l| l.get()).wrapping_sub(room::get())
}

/// Collect once this thread holds `limit` bytes.
fn set_limit(limit: isize) {
    let held = held();
    LIMIT.with(|l| l.set(limit));
    room::set(limit.wrapping_sub(held));
}

/// Start collecting automatically once the session is up (base and the
/// default packages loaded), on the interpreter's thread. The program's
/// heap starts at `R_VSIZE` above what that took.
pub fn enable() {
    if std::env::var("ROBIGO_CYCLES").is_ok_and(|v| v == "0") {
        return;
    }
    if let Some(n) = std::env::var("ROBIGO_CYCLES_TORTURE").ok().and_then(|v| v.parse::<u64>().ok()) {
        if n > 0 {
            TORTURE.with(|t| t.set(n));
            set_limit(held() - OFF);
            return;
        }
    }
    let boot = held();
    BOOT.with(|b| b.set(boot));
    set_limit(boot + V_SIZE.with(|v| v.get()) as isize);
}

/// Size the heap for the `held` bytes a collection left.
fn adjust(held: isize) {
    if TORTURE.with(|t| t.get()) > 0 {
        set_limit(held - OFF);
        return;
    }
    let boot = BOOT.with(|b| b.get());
    let live = (held - boot).max(0) as f64;
    let v_size = next_size(live, V_SIZE.with(|v| v.get()));
    V_SIZE.with(|v| v.set(v_size));
    // (more than is held: the size leaves at least R_VSIZE * MIN_FREE_FRAC free)
    set_limit(boot + v_size as isize);
}

/// `R_VSize` after a collection that left `live` bytes in a heap of
/// `v_size` (`size_needed` is 0: no allocation is waiting).
// R: memory.c:AdjustHeapSize (the vector heap)
fn next_size(live: f64, mut v_size: f64) -> f64 {
    let min_v_free = R_VSIZE * MIN_FREE_FRAC;
    let v_needed = live + min_v_free;
    let vect_occup = v_needed / v_size;
    if vect_occup > 1.0 {
        v_size = v_needed;
    }
    if vect_occup > V_GROW_FRAC {
        v_size += V_GROW_INCR_MIN + V_GROW_INCR_FRAC * v_size;
    } else if vect_occup < V_SHRINK_FRAC {
        v_size -= V_SHRINK_INCR_MIN + V_SHRINK_INCR_FRAC * v_size;
        if v_size < v_needed {
            v_size = v_needed;
        }
        if v_size < R_VSIZE {
            v_size = R_VSIZE;
        }
    }
    v_size
}

/// A safe point found the heap over its limit: run `gc()`'s cycle steps (again if the first round released
/// objects only C held, so that what they held goes in the same
/// collection), then size the heap for what is left.
/// The bridge's round runs the pending finalizers, as GNU R does at these
/// points (`R_RunPendingFinalizers`).
// R: eval.c:bc_check_sigint (R_RunPendingFinalizers), memory.c:R_gc_internal
#[cold]
#[inline(never)]
pub fn collect(interp: &mut Interp) {
    let every = TORTURE.with(|t| t.get());
    if every > 1
        && TORTURE_SEEN.with(|c| {
            c.set(c.get() + 1);
            c.get() % every != 0
        })
    {
        return;
    }
    // Not while a native routine runs (R code it called reached this
    // point): until the routine returns, its writes to the objects it holds
    // are not yet the runtime's (the bridge copies them back then), so the
    // two sides disagree on what is reachable. The collection stays due
    // and runs at the first safe point after the routine returns.
    if crate::native::bridge().is_some_and(|b| b.in_native_call()) {
        return;
    }
    if COLLECTING.with(|c| c.replace(true)) {
        return;
    }
    let before = held();
    let t0 = std::time::Instant::now();
    // (as gc(): implementation caches are weak in R, see misc.rs do_gc)
    if !interp.fun_cache.is_empty() {
        interp.fun_cache.clear();
    }
    crate::vm::clear_gc_caches_running();
    match crate::native::bridge() {
        Some(b) => {
            if b.collect_cycles(interp) {
                b.collect_cycles(interp);
            }
        }
        None => {
            crate::reachability::reclaim_cycles();
        }
    }
    // R: connections.c:conFinalizer runs at any collection: an unused
    // connection closes (with its warning) here too, not only at gc().
    // Its error (a warning under options(warn = 2)) is reported as
    // RunFinalizers reports a finalizer's, without the caller's handlers,
    // and the evaluation goes on.
    if let Err(crate::error::Flow::Error(e)) = crate::builtins::io::close_unused_connections(interp) {
        if !e.signaled {
            let saved = interp.handlers.take();
            let _ = interp.signal_error(e);
            interp.handlers = saved;
        }
    }
    let after = held();
    adjust(after);
    if std::env::var_os("ROBIGO_CYCLES_LOG").is_some() {
        eprintln!(
            "[cycles] heap {} -> {} MB, next at {} MB, {:.1} ms",
            before >> 20,
            after >> 20,
            LIMIT.with(|l| l.get()).max(0) >> 20,
            t0.elapsed().as_secs_f64() * 1e3
        );
    }
    COLLECTING.with(|c| c.set(false));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_collection_is_due_while_the_thread_holds_the_limit() {
        // (the counter is this test thread's: the test harness's allocator
        // does not report, so only these calls move it)
        assert_eq!(held(), 0);
        set_limit(100 << 20);
        allocated(150 << 20);
        assert!(due());
        // a temporary freed before the safe point: nothing is due there
        freed(150 << 20);
        assert!(!due());
        // what is held is counted net, however it was allocated
        allocated(60 << 20);
        allocated(60 << 20);
        freed(30 << 20);
        assert_eq!(held(), 90 << 20);
        assert!(!due());
        // due once over the limit, as R_VSize is a size the heap may fill
        allocated(10 << 20);
        assert!(!due());
        allocated(1);
        assert!(due());
        // a new limit keeps what is held
        set_limit(200 << 20);
        assert_eq!(held(), (100 << 20) + 1);
        assert!(!due());
    }

    #[test]
    fn the_heap_grows_as_gnu_rs_vector_heap() {
        // a 200 MB live heap: R_VSize becomes what is needed, then grows by
        // the increment (the occupancy is over R_VGrowFrac)
        let live = (200 << 20) as f64;
        let needed = live + R_VSIZE * MIN_FREE_FRAC;
        let v = next_size(live, R_VSIZE);
        assert_eq!(v, needed + V_GROW_INCR_MIN + V_GROW_INCR_FRAC * needed);
        // most of it freed: the size shrinks, not below R_VSIZE
        let mut v = v;
        for _ in 0..100 {
            v = next_size((1 << 20) as f64, v);
        }
        assert_eq!(v, R_VSIZE);
        // in between: unchanged
        assert_eq!(next_size(0.5 * 100e6 - R_VSIZE * MIN_FREE_FRAC, 100e6), 100e6);
    }
}
