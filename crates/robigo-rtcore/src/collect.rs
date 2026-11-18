//! Where collections happen (docs/g3.md §3 items 2 to 4): safepoints, the
//! per-chunk poll of a Rust loop, and allocation that may collect first.
//! Every one takes `&mut Cx`, so no `Ref`, view or slice is alive across it,
//! and collects with the isolate's precise roots.
//!
//! Allocation under `&Cx` never collects (H2's poll-only path): when it
//! crosses GNU R's trigger it sets `GC_DUE` in the isolate's poll word, and
//! the next safepoint serves it. A request of [`LARGE`] bytes or more goes
//! through [`Cx::alloc_rooted`] instead, which collects before allocating as
//! GNU R does; a `&Cx` one made while a collection is due is counted
//! (`GcStats::large_while_due`) and panics under `ROBIGO_GC_STATS=check`.

#![allow(unsafe_code)]

use crate::exit::RResult;
use crate::fresh::FreshVec;
use crate::handle::Elt;
use crate::isolate::Cx;
use robigo_heap::{GC_DUE, GcStats, LARGE, Safepoint};
use std::sync::atomic::Ordering;

/// [`Cx::refuse_first`]'s guard.
///
/// Attempts cannot nest: a build's closure gets `&Cx`, and
/// [`Cx::build_with`] and [`Cx::retry_alloc`] take `&mut Cx`, so no attempt
/// starts while another's guard lives, and this drop's disarm ends only
/// its own attempt's refusal.
#[cfg(feature = "retry-torture")]
#[must_use = "the refusal ends when this guard drops; dropping it at once makes it a no-op"]
pub(crate) struct RefuseFirst<'c>(&'c Cx);

#[cfg(feature = "retry-torture")]
impl Drop for RefuseFirst<'_> {
    fn drop(&mut self) {
        self.0.refuse_next_alloc(false);
    }
}

impl Cx {
    /// Whether a collection is due: served at the next safepoint.
    #[inline(always)]
    pub fn gc_due(&self) -> bool {
        self.iso().poll.load(Ordering::Relaxed) & GC_DUE != 0
    }

    /// Whether the next safepoint has work to do (any bit of the poll
    /// word): a caller that would skip one gives up instead.
    #[inline(always)]
    pub fn safepoint_pending(&self) -> bool {
        self.iso().poll.load(Ordering::Relaxed) != 0
    }

    /// A safepoint of kind `kind` (docs/g3.md §3 item 2: R entries, VM
    /// back-edges and calls, builtin return, ...): one load of the poll
    /// word, and when a bit is set, the collection that is due, with the
    /// isolate's roots. `Err` is reserved for the interrupt that will share
    /// the word (GNU R's `R_CheckUserInterrupt` at the same points).
    #[inline(always)]
    pub fn safepoint(&mut self, kind: Safepoint) -> RResult<()> {
        if self.iso().poll.load(Ordering::Relaxed) != 0 { self.serve_poll(kind) } else { Ok(()) }
    }

    #[cold]
    #[inline(never)]
    fn serve_poll(&mut self, kind: Safepoint) -> RResult<()> {
        let iso = self.iso();
        // SAFETY: `&mut Cx` has ended every `Ref`, view and slice of this
        // capability, so everything Rust code still needs is in a root of
        // this isolate; every root edge is null, immortal or a live object of
        // this heap (writes into roots are owner-checked); no other reference
        // into the core or the roots is live, and the collection reaches
        // nothing of ours (finalizers are only queued).
        unsafe { iso.core().safepoint(kind, iso.roots_mut()) };
        Ok(())
    }

    /// The poll of a Rust loop whose allocation is not bounded by a constant
    /// (docs/g3.md §3 item 4): call it after each chunk, with the loop's
    /// inputs and partial results in roots, and reacquire every slice after
    /// it. `bytes` is the chunk's bound, at most what the loop allocates
    /// between two polls: E4 holds the loop to it, and it stays below
    /// [`LARGE`] (asserted in debug builds), since a larger chunk is one
    /// allocation, which [`Cx::alloc_rooted`] serves.
    #[inline(always)]
    pub fn poll_alloc(&mut self, bytes: usize) -> RResult<()> {
        debug_assert!(bytes < LARGE, "robigo: a poll_alloc chunk of {bytes} bytes (at most LARGE; use alloc_rooted)");
        self.safepoint(Safepoint::PollAlloc)
    }

    /// A vector of `n` elements allocated where a collection may run first
    /// (docs/g3.md §3 item 3): collect if one is due or if this request makes
    /// one due, check GNU R's limits, then allocate, as GNU R's
    /// `allocVector3` collects before it allocates. For requests of [`LARGE`]
    /// bytes or more; any size works. Plain data is zeroed, CHARSXP elements
    /// are `""`, list elements R_NilValue. `Err` is GNU R's error ("vector
    /// memory limit ... reached", "cannot allocate vector of size ...").
    ///
    /// The capability stays borrowed while the vector is filled:
    /// [`FreshVec::cx`] gives the shared one for the `&Cx` reads of the fill.
    ///
    /// ```
    /// # use robigo_rtcore::OwnedIsolate;
    /// # let mut iso = OwnedIsolate::new();
    /// # let cx = iso.cx();
    /// cx.scope(|cx, s| {
    ///     let out = s.out(cx);
    ///     let mut v = cx.alloc_rooted::<f64>(1 << 18)?; // 2 MiB: may collect first
    ///     let cx = v.cx();
    ///     v.as_mut_slice().iter_mut().enumerate().for_each(|(i, e)| *e = i as f64);
    ///     out.set(cx, v.finish());
    ///     Ok::<_, robigo_rtcore::RError>(())
    /// })
    /// .unwrap();
    /// ```
    pub fn alloc_rooted<'g, T: Elt<'g>>(&'g mut self, n: usize) -> RResult<FreshVec<'g, T>> {
        let r = {
            let iso = self.iso();
            // SAFETY: as in `serve_poll`: a collection may run here, with the
            // isolate's roots, while `&mut Cx` has ended every `Ref`.
            unsafe { iso.core().alloc_rooted(T::SEXPTYPE, n, iso.roots_mut()) }
        };
        match r {
            // SAFETY: fresh, of this heap, a vector of `T`'s type and length `n`.
            Ok(x) => Ok(unsafe { FreshVec::from_fresh(x, n, &*self) }),
            Err(e) => Err(self.error_call(None, &e.message())),
        }
    }

    /// The collection trigger's state in GNU R's units (`R_NSize`,
    /// `R_VSize` and what is in use): what `gc()` reports.
    pub fn heap_report(&self) -> robigo_heap::HeapReport {
        // SAFETY: one read of the core, which calls nothing of ours.
        unsafe { self.iso().core() }.heap_sizes()
    }

    /// `gc(reset = TRUE)`'s reset of the most-used counts.
    pub fn reset_maxused(&mut self) {
        // SAFETY: one call into the core, which calls nothing of ours.
        unsafe { self.iso().core() }.reset_maxused()
    }

    /// GNU R's `R_gc_torture(gap, wait, FALSE)`: a collection due every
    /// `gap` allocations, the first after `wait` of them (`gap` 0 turns it
    /// off); each is served at the next safepoint (allocation never
    /// collects).
    // R: memory.c:R_gc_torture
    pub fn gc_torture(&mut self, gap: i32, wait: i32) {
        // SAFETY: one call into the core, which calls nothing of ours.
        unsafe { self.iso().core() }.gc_torture(gap, wait)
    }

    /// `R_GetMaxVSize()`: the vector memory limit in bytes, None when
    /// there is none.
    // R: memory.c:R_GetMaxVSize
    pub fn max_vsize(&self) -> Option<usize> {
        // SAFETY: one read of the core.
        unsafe { self.iso().core() }.max_vsize()
    }

    /// `R_SetMaxVSize(bytes)`: refused (false, nothing changed) below the
    /// vector heap's current size; None lifts the limit.
    // R: memory.c:R_SetMaxVSize
    pub fn set_max_vsize(&mut self, bytes: Option<usize>) -> bool {
        // SAFETY: one call into the core, which calls nothing of ours.
        unsafe { self.iso().core() }.set_max_vsize(bytes)
    }

    /// Start E9's allocation-failure knob (`ROBIGO_FAIL_ALLOC_AT=n`) if
    /// the environment set it: from here the `n`th allocation is refused
    /// once, and every refusal takes the retry path of the code that made it
    /// (docs/g3.md §3 item 5: no committed effect is replayed).
    pub fn arm_fail_alloc(&mut self) {
        // SAFETY: one call into the core, which calls nothing of ours.
        unsafe { self.iso().core() }.arm_fail_alloc()
    }

    /// [`Cx::retry_torture`]'s step: E9's knob set to refuse the next
    /// allocation once (`on`), or off.
    #[cfg(feature = "retry-torture")]
    fn refuse_next_alloc(&self, on: bool) {
        // SAFETY: one call into the core, which calls nothing of ours.
        unsafe { self.iso().core() }.fail_alloc_at(on as u64)
    }

    /// [`Cx::retry_torture`] for one attempt: the next allocation is
    /// refused while the guard lives, and the knob is off once it drops,
    /// after an attempt that panicked too.
    #[cfg(feature = "retry-torture")]
    fn refuse_first(&self) -> RefuseFirst<'_> {
        self.refuse_next_alloc(true);
        RefuseFirst(self)
    }

    /// The guard of a build's attempt under [`Cx::retry_torture`]:
    /// [`Cx::refuse_first`]'s for the first attempt (`first`, then false),
    /// none for the retries. `build_with` and `retry_alloc` share it.
    #[cfg(feature = "retry-torture")]
    pub(crate) fn refuse_once(&self, first: &mut bool) -> Option<RefuseFirst<'_>> {
        std::mem::take(first).then(|| self.refuse_first())
    }

    /// Whether E9's knob is set (`ROBIGO_FAIL_ALLOC_AT` as the heap read
    /// it, armed or not yet): [`Cx::set_retry_torture`] would take it over.
    pub(crate) fn fail_alloc_set(&self) -> bool {
        // SAFETY: one read of the core.
        unsafe { self.iso().core() }.fail_at() != 0
    }

    /// Whether gctorture is on (`gctorture()`'s result).
    /// Why the last refusal was made, if it was a limit or a page the
    /// system refused (then cleared): [`Cx::refused_again`]'s error.
    pub(crate) fn take_refusal(&mut self) -> Option<robigo_heap::AllocError> {
        // SAFETY: one call into the core, which calls nothing of ours.
        unsafe { self.iso().core() }.take_refusal()
    }

    /// Refuse every new heap chunk from now on (`on`), as a system with no
    /// memory left would: a test knob for the refusal paths.
    pub fn set_refuse_chunks(&mut self, on: bool) {
        // SAFETY: one call into the core, which calls nothing of ours.
        unsafe { self.iso().core() }.set_refuse_chunks(on)
    }

    pub fn gc_torture_on(&self) -> bool {
        // SAFETY: one read of the core.
        unsafe { self.iso().core() }.torture()
    }

    /// The heap's counters (collections per safepoint kind, E4's overshoot,
    /// `large_while_due`, ...).
    pub fn gc_stats(&self) -> GcStats {
        // SAFETY: one read of the core, which calls nothing of ours.
        unsafe { self.iso().core() }.stats.clone()
    }
}
