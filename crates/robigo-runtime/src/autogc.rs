//! Automatic collection of the runtime's reference cycles (#302).
//!
//! Values are reference counted, so what a cycle holds (an environment and
//! the closures or promises in it that point back) is freed only by a
//! collection that traces it: `gc()`'s cycle steps (the native bridge's, or
//! [`crate::reachability::reclaim_cycles`] without one). GNU R collects when
//! its heap is full, so a program never has to call `gc()` to get its
//! memory back; here a collection is due when the bytes the runtime has
//! allocated reach a threshold sized as GNU R sizes its vector heap
//! (memory.c's `R_VSize` and `AdjustHeapSize`).
//!
//! A due collection runs only where GNU R runs pending finalizers, its
//! interrupt-check points: closure calls (the bytecode's
//! `START_BCFRAME_CALL`) and loop back edges (`GOTO`, `BRIFNOT`, `STEPFOR`;
//! the AST loops' iterations), eval.c's `bc_check_sigint`. Arbitrary code
//! can run there in GNU R; never at a native call's exit.

use std::cell::Cell;
use std::sync::atomic::{AtomicIsize, Ordering::Relaxed};

use crate::interp::Interp;

/// Bytes the global allocator has handed out and not taken back. The
/// executable's allocator keeps it ([`note_alloc`], [`note_free`]); in a
/// process whose allocator does not, it stays 0 and nothing is collected.
static HEAP_BYTES: AtomicIsize = AtomicIsize::new(0);

/// The heap size at which a collection is due (`R_VSize`, in bytes):
/// unreachable until [`enable`].
static TRIGGER: AtomicIsize = AtomicIsize::new(isize::MAX);

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
    /// Collect at every safe point (`ROBIGO_CYCLES_TORTURE`).
    static TORTURE: Cell<bool> = const { Cell::new(false) };
}

/// The global allocator handed out `n` bytes.
#[inline(always)]
pub fn note_alloc(n: usize) {
    // (A load and a store, not an atomic add: the R thread allocates
    // nearly everything, and a lost update from another thread only moves
    // the next collection a little.)
    HEAP_BYTES.store(HEAP_BYTES.load(Relaxed).wrapping_add(n as isize), Relaxed);
}

/// The global allocator took back `n` bytes.
#[inline(always)]
pub fn note_free(n: usize) {
    HEAP_BYTES.store(HEAP_BYTES.load(Relaxed).wrapping_sub(n as isize), Relaxed);
}

/// The bytes allocated and not freed.
pub fn heap_bytes() -> isize {
    HEAP_BYTES.load(Relaxed)
}

/// Is a collection due? (The check at every safe point.)
#[inline(always)]
pub fn due() -> bool {
    HEAP_BYTES.load(Relaxed) >= TRIGGER.load(Relaxed)
}

/// Start collecting automatically, once the base environment is loaded:
/// the heap it took is sized as if a collection had just found it live.
pub fn enable() {
    if std::env::var("ROBIGO_CYCLES").is_ok_and(|v| v == "0") {
        return;
    }
    if std::env::var_os("ROBIGO_CYCLES_TORTURE").is_some() {
        TORTURE.with(|t| t.set(true));
        TRIGGER.store(0, Relaxed);
        return;
    }
    adjust();
}

/// Size the heap for what a collection left.
fn adjust() {
    if TORTURE.with(|t| t.get()) {
        TRIGGER.store(0, Relaxed);
        return;
    }
    let live = HEAP_BYTES.load(Relaxed).max(0) as f64;
    let v_size = next_size(live, V_SIZE.with(|v| v.get()));
    V_SIZE.with(|v| v.set(v_size));
    TRIGGER.store(v_size as isize, Relaxed);
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
    if COLLECTING.with(|c| c.replace(true)) {
        return;
    }
    // (as gc(): implementation caches are weak in R, see misc.rs do_gc)
    if !interp.fun_cache.is_empty() {
        interp.fun_cache.clear();
    }
    crate::vm::clear_gc_caches();
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
    adjust();
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
