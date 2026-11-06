//! Automatic collection of the runtime's reference cycles (#302).
//!
//! Values are reference counted, so what a cycle holds (an environment and
//! the closures or promises in it that point back) is freed only by a
//! collection that traces it: `gc()`'s cycle steps (the native bridge's, or
//! [`crate::reachability::reclaim_cycles`] without one). GNU R collects when
//! its heap is full, so a program never has to call `gc()` to get its
//! memory back; here a collection is due when the heap (the allocator's
//! committed bytes, measured off the allocation fast path) reaches a
//! threshold sized as GNU R sizes its vector heap (memory.c's `R_VSize`
//! and `AdjustHeapSize`).
//!
//! A due collection runs only where GNU R runs pending finalizers, its
//! interrupt-check points: closure calls (the bytecode's
//! `START_BCFRAME_CALL`) and loop back edges (`GOTO`, `BRIFNOT`, `STEPFOR`;
//! the AST loops' iterations), eval.c's `bc_check_sigint`. Arbitrary code
//! can run there in GNU R; never at a native call's exit.

use std::cell::Cell;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering::Relaxed};

use crate::interp::Interp;

/// The heap's size in bytes, as the executable's allocator reports it
/// ([`set_heap_probe`]); without one nothing is collected.
static PROBE: OnceLock<fn() -> usize> = OnceLock::new();

/// The heap size at which a collection is due (`R_VSize`, in bytes):
/// unreachable until [`enable`].
static TRIGGER: AtomicIsize = AtomicIsize::new(isize::MAX);

/// A collection is due at the next safe point.
static DUE: AtomicBool = AtomicBool::new(false);

/// The heap once the session was up (base and the default packages
/// loaded): GNU R's sizes apply to what the program allocates beyond it
/// (GNU R's own base takes a few megabytes of its heap, robigo's
/// environments and compiled code tens).
static BOOT: AtomicIsize = AtomicIsize::new(0);

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

/// How the executable measures the heap (mimalloc's committed bytes).
pub fn set_heap_probe(probe: fn() -> usize) {
    let _ = PROBE.set(probe);
}

/// The heap's size now (0 without a probe).
pub fn heap_bytes() -> isize {
    PROBE.get().map_or(0, |p| p() as isize)
}

/// The allocator's periodic call (mimalloc's deferred-free hook, every
/// hundred allocations that leave its fast path): the heap is measured
/// there, so the allocations themselves pay nothing. It only notes that a
/// collection is due; the next safe point runs it.
// R: memory.c:allocVector3's `VHEAP_FREE() < alloc_size`
pub fn heartbeat() {
    // (not while a collection runs: building its graph allocates, and the
    // heap is sized for what is left once it is done)
    if !DUE.load(Relaxed) && !COLLECTING.try_with(|c| c.get()).unwrap_or(true) && heap_bytes() >= TRIGGER.load(Relaxed)
    {
        DUE.store(true, Relaxed);
    }
}

/// Is a collection due? (The check at every safe point.)
#[inline(always)]
pub fn due() -> bool {
    DUE.load(Relaxed)
}

/// Start collecting automatically, once the session is up (base and the
/// default packages loaded):
/// the program's heap starts at `R_VSIZE` above what that took.
pub fn enable() {
    if std::env::var("ROBIGO_CYCLES").is_ok_and(|v| v == "0") {
        return;
    }
    if let Some(n) = std::env::var("ROBIGO_CYCLES_TORTURE").ok().and_then(|v| v.parse::<u64>().ok()) {
        if n > 0 {
            TORTURE.with(|t| t.set(n));
            TRIGGER.store(0, Relaxed);
            DUE.store(true, Relaxed);
            return;
        }
    }
    BOOT.store(heap_bytes(), Relaxed);
    TRIGGER.store(BOOT.load(Relaxed) + V_SIZE.with(|v| v.get()) as isize, Relaxed);
}

/// Size the heap for what a collection left.
fn adjust() {
    if TORTURE.with(|t| t.get()) > 0 {
        TRIGGER.store(0, Relaxed);
        DUE.store(true, Relaxed);
        return;
    }
    let boot = BOOT.load(Relaxed);
    let live = (heap_bytes() - boot).max(0) as f64;
    let v_size = next_size(live, V_SIZE.with(|v| v.get()));
    V_SIZE.with(|v| v.set(v_size));
    TRIGGER.store(boot + v_size as isize, Relaxed);
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

/// A safe point found a collection due: run `gc()`'s cycle steps (again
/// if the first round released objects only C held, so that what they
/// held goes in the same collection), then size the heap for what is left.
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
    let log = std::env::var_os("ROBIGO_CYCLES_LOG").is_some();
    let (t0, before) = (std::time::Instant::now(), if log { heap_bytes() } else { 0 });
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
    DUE.store(false, Relaxed);
    adjust();
    if log {
        eprintln!(
            "[cycles] heap {} -> {} MB, next at {} MB, {:.1} ms",
            before >> 20,
            heap_bytes() >> 20,
            TRIGGER.load(Relaxed) >> 20,
            t0.elapsed().as_secs_f64() * 1e3
        );
    }
    COLLECTING.with(|c| c.set(false));
}

#[cfg(test)]
mod tests {
    use super::*;

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
