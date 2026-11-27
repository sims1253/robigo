//! A character vector built element by element where a collection may run
//! between two elements (docs/g3.md §3 item 4): a builtin whose loop makes
//! a CHARSXP per element (paste's), and so allocates without a bound a
//! constant names, has GNU R's collections inside its loop, where
//! `allocVector` collects in the allocation that crosses the trigger.
//!
//! [`Cx::fill_strings`] runs the builtin's loop under one `&Cx`, as
//! [`Cx::build_with`] does; the loop stores each element with
//! [`Fill::put`], which then reads the poll word: the allocation that
//! crosses the trigger sets `GC_DUE` in it (every collector mode), as an
//! interrupt sets its bit. When a bit is set, `put` says stop, the loop
//! returns, and the fill serves the safepoint ([`Safepoint::Loop`], counted
//! with the VM's back-edges) with the partial result in `out`, then runs
//! the loop again from the next element. A run that never collects pays one
//! load of the poll word and a branch per element. An interrupt is served at
//! the next element, as the VM serves one at its next back-edge.
//!
//! The builtin keeps its loop, so what it computes once per call (the
//! separator's translation, its marks) stays in its own locals.
//!
//! What the fill promises, so a safepoint between two elements is legal
//! (H's four, with the tests that check them):
//!
//! - (a) After a safepoint the result is reached only through its root,
//!   read again: no `Sexp` or data pointer of it is kept across the serve
//!   (a moving collector may have moved it).
//! - (b) The loop is `for<'a> FnMut(&'a Cx, &mut Fill<'a>)`: the views it
//!   makes of its inputs borrow that `&Cx`, which the fill gives each pass
//!   anew, so none can be kept from one pass to the next (the compiler
//!   refuses it: below); each pass makes them again from roots. The serve
//!   runs only between passes, after the loop has returned.
//!
//! ```
//! # use robigo_rtcore::{Char, OwnedIsolate};
//! # let mut iso = OwnedIsolate::new();
//! # let cx = iso.cx();
//! cx.scope(|cx, s| {
//!     let out = s.out(cx);
//!     cx.fill_strings(out, 3, |cx, fill| {
//!         for i in fill.start()..3 {
//!             if fill.put(i, cx.mk_char(format!("e{i}").as_bytes(), robigo_rtcore::Encoding::Native))? {
//!                 return Ok(Ok(()));
//!             }
//!         }
//!         Ok(Ok(()))
//!     })
//! })
//! .unwrap();
//! ```
//!
//! A view kept from one pass for the next does not compile:
//!
//! ```compile_fail
//! # use robigo_rtcore::{Char, OwnedIsolate};
//! # let mut iso = OwnedIsolate::new();
//! # let cx = iso.cx();
//! cx.scope(|cx, s| {
//!     let out = s.out(cx);
//!     let mut kept: Option<Char<'_>> = None;
//!     cx.fill_strings(out, 1, |cx, fill| {
//!         let c = kept.unwrap_or_else(|| cx.blank_string());
//!         kept = Some(c);
//!         fill.put(0, Ok(c)).map(|_| Ok(()))
//!     })
//! })
//! .unwrap();
//! ```
//! - (c) Every element not yet filled is `""` (`R_BlankString`, immortal)
//!   from allocation on, so the collector traces a valid vector at every
//!   serve.
//! - (d) After a serve, each element is stored with `set_elt`, through the
//!   barrier and the counts (the vector may be old by then). The first pass
//!   stores as [`FreshVec::set`] does, without the barrier, which is right
//!   for every collector (H): the barrier's slow path matters only for an
//!   object with BARRIER or FROZEN set, and a vector the builtin allocates
//!   has neither (born after the boot boundary, never immortal); no
//!   collection runs during a pass, so within the first pass it cannot be
//!   marked, moved, frozen or logged. Rooting decides reachability, not the
//!   barrier. The one condition is the reference counts: each element is
//!   counted, and there is no old value to drop ("" is immortal), as
//!   `FreshVec::set` does.
//!
//! A builtin's loop must end its pass at `put`'s stop: a `put` after the
//! stop panics in debug builds. Going on is not unsound, but the collection
//! waits for the pass to end, which E4's overshoot bound shows, and under
//! `ROBIGO_GC_STATS=check` a `&Cx` request of `LARGE` bytes or more made
//! while a collection is due panics where it is made, a misleading failure.
//!
//! Which converted builtin owes its own collecting case (H): the type-level
//! rule and the `compile_fail` test below cover the views, and
//! `tests/rt/strings/paste-collect.R`, under gctorture and the young-moving
//! collector, covers this module's serve and re-read. A builtin whose loop
//! only makes CHARSXPs from views (`mk_char` of a buffer it fills) rides on
//! that case. One whose loop does more owes a case of its own, long enough
//! to collect inside one call: anything else that allocates (sprintf's and
//! format's formatting buffers, coerce's list paths) or a refusal-retry
//! path of its own.
//!
//! A conversion also makes existing cases with long per-element loops
//! collect at every element under torture, as GNU R's do: each one reports
//! the per-case seconds of rt-ci's two torture steps (gctorture, and
//! young:16 with it) before and after, and fixes its own outliers before
//! it is filed, as the paste conversion did (`tests/rt/env/large-frames.R`
//! makes its names outside torture).

#![allow(unsafe_code)]

use crate::exit::RResult;
use crate::fresh::FreshVec;
use crate::handle::{Char, Ref, Sym};
use crate::isolate::Cx;
use crate::outcome::{Built, Diag};
use crate::root::{Out, Root};
use robigo_heap::types::{EXPRSXP, STRSXP, VECSXP};
use robigo_heap::{NeedGc, Safepoint, Sexp};
use std::sync::atomic::{AtomicU32, Ordering};

/// What a pass of the builtin's loop ends with: done or stopped by
/// [`Fill::put`] / [`FillList::put`] (`Ok(Ok(()))`), an error to raise
/// (the partial result is dropped), or an allocation the heap refused
/// (`Err`; refused again, it is GNU R's error for the cause). Two refusals
/// are the same to the loop, which returns either with `?`:
/// - the one given to [`Fill::put`] or [`FillList::put`], for that
///   element's allocation: the fill collects and runs the loop again from
///   that element;
/// - one the loop met in an allocation of its own, outside `put` (a
///   coercion in its setup, a buffer): the fill collects and runs the pass
///   again from its start, `fill.start()` (0 for the first pass). The
///   elements the pass had put are made again, with the same values.
pub type FillPass<'r> = Result<Result<(), Diag<'r>>, NeedGc>;

/// Whether [`Cx::fill_list`] builds a `VECSXP` (list) or an `EXPRSXP`
/// (expression vector).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    List,
    Expr,
}

impl ListKind {
    const fn sexptype(self) -> u8 {
        match self {
            ListKind::List => VECSXP,
            ListKind::Expr => EXPRSXP,
        }
    }
}

/// Where a pass of [`Cx::fill_strings`]'s loop stores its elements.
pub struct Fill<'a> {
    into: Into<'a>,
    /// The first element of this pass.
    start: usize,
    /// Where the pass stopped, set by [`Fill::put`] when it stops it (`n`
    /// until then: a pass that returns without a stop made them all).
    next: usize,
    n: usize,
    poll: &'a AtomicU32,
    cx: &'a Cx,
    /// (debug builds) the next element `put` expects: a loop that skips
    /// one, or returns before a stop with elements unmade, panics.
    #[cfg(debug_assertions)]
    expect: usize,
    /// (debug builds) `put` has said stop: another `put` this pass panics.
    /// Not for correctness (each later `put` sets the resume point again,
    /// so nothing is skipped or made twice): for the collection the loop
    /// delays, which E4's overshoot bound would show, and for
    /// `ROBIGO_GC_STATS=check`'s panic at a `&Cx` request of `LARGE` bytes or
    /// more made while it is due, which would fail CI misleadingly.
    #[cfg(debug_assertions)]
    stopped: bool,
}

enum Into<'a> {
    /// The first pass: the fresh vector.
    Fresh(FreshVec<'a, Char<'a>>),
    /// A pass after a serve: the vector, read again from its root.
    Rooted(Sexp),
}

impl<'a> Fill<'a> {
    /// The element this pass starts at: the loop makes elements
    /// `start()..len`.
    #[inline(always)]
    pub fn start(&self) -> usize {
        self.start
    }

    /// Element `i` (each in turn from `start()`) is `c`, the CHARSXP made
    /// for it: stored, then the poll word read. `Ok(true)`: stop the pass
    /// now and return `Ok(Ok(()))` (a collection or an interrupt is due);
    /// the fill serves it and runs the loop again from `i + 1`. `Err`: the
    /// heap refused `c`'s allocation; return it (`?`), and the fill collects
    /// and runs the loop again from `i`.
    #[inline(always)]
    pub fn put(&mut self, i: usize, c: Result<Char<'a>, NeedGc>) -> Result<bool, NeedGc> {
        #[cfg(debug_assertions)]
        {
            assert!(!self.stopped, "robigo: Fill::put after the stop: the pass did not end at the stop");
            assert_eq!(i, self.expect, "robigo: Fill::put of element {i}, expected {}", self.expect);
            self.expect += 1;
        }
        let c = match c {
            Ok(c) => c,
            Err(e) => return Err(self.refused(i, e)),
        };
        match &mut self.into {
            Into::Fresh(v) => v.set(i, c),
            Into::Rooted(x) => {
                assert!(i < self.n, "robigo: element {i} of {}", self.n);
                let c = c.as_ref().sexp();
                self.cx.iso().check_owns(c);
                // (d) SAFETY: element `i < n` of the live STRSXP read from
                // the result's root this pass, and a live CHARSXP of this
                // heap; the store keeps the barrier and the counts.
                unsafe { x.set_elt(i, c) };
            }
        }
        if self.poll.load(Ordering::Relaxed) != 0 && i + 1 < self.n {
            self.next = i + 1;
            #[cfg(debug_assertions)]
            {
                self.stopped = true;
            }
            return Ok(true);
        }
        Ok(false)
    }

    #[cold]
    fn refused(&mut self, i: usize, e: NeedGc) -> NeedGc {
        self.next = i;
        e
    }

    /// (debug builds) a pass that returned `Ok(Ok(()))` without a stop made
    /// every element.
    #[inline(always)]
    fn check_done(&self, pass: &FillPass<'_>) {
        #[cfg(debug_assertions)]
        if matches!(pass, Ok(Ok(()))) && self.next >= self.n {
            assert_eq!(self.expect, self.n, "robigo: a fill pass returned with elements unmade");
        }
        let _ = pass;
    }
}

/// Where a pass of [`Cx::fill_list`]'s loop stores its elements.
///
/// Follows H's rules for list/expression fills:
/// - **Rule 1:** a `&Cx` request of `LARGE` (1 MiB) or more made while a
///   collection is due is a defect (panics under `ROBIGO_GC_STATS=check`).
///   An element that may need such a request checks [`Cx::gc_due`] first
///   and, if due, ends the pass with `put(i, Err(NeedGc::new(sexptype, len)))`.
/// - **Rule 2:** make the whole element, then pass it to [`FillList::put`].
///
/// A view kept from one pass for the next does not compile:
///
/// ```compile_fail
/// # use robigo_rtcore::{ListKind, OwnedIsolate, Ref};
/// # let mut iso = OwnedIsolate::new();
/// # let cx = iso.cx();
/// cx.scope(|cx, s| {
///     let out = s.out(cx);
///     let mut kept: Option<Ref<'_>> = None;
///     cx.fill_list(out, ListKind::List, 1, |cx, fill| {
///         let v = kept.unwrap_or_else(|| cx.nil());
///         kept = Some(v);
///         fill.put(0, Ok(v)).map(|_| Ok(()))
///     })
/// })
/// .unwrap();
/// ```
pub struct FillList<'a> {
    into: IntoList<'a>,
    start: usize,
    next: usize,
    n: usize,
    poll: &'a AtomicU32,
    cx: &'a Cx,
    #[cfg(debug_assertions)]
    expect: usize,
    #[cfg(debug_assertions)]
    stopped: bool,
}

enum IntoList<'a> {
    /// The first pass: the fresh list/expression vector (no BARRIER/FROZEN
    /// bit; every unfilled slot holds immortal `R_NilValue`).
    Fresh(FreshVec<'a, Ref<'a>>),
    /// A pass after a serve: the vector, read again from its root (may be
    /// old under `young:16`, so stores go through `set_elt`).
    Rooted(Sexp),
}

impl<'a> FillList<'a> {
    /// The element this pass starts at: the loop makes elements
    /// `start()..len`.
    #[inline(always)]
    pub fn start(&self) -> usize {
        self.start
    }

    /// Element `i` (each in turn from `start()`) is `v`, the completed
    /// element made for it (Rule 2): stored, then the poll word read.
    /// `Ok(true)`: stop the pass now and return `Ok(Ok(()))`; the fill
    /// serves the safepoint and runs the loop again from `i + 1`. `Err`:
    /// the heap refused `v`'s allocation; return it (`?`), and the fill
    /// collects and runs the loop again from `i`.
    #[inline(always)]
    pub fn put(&mut self, i: usize, v: Result<Ref<'a>, NeedGc>) -> Result<bool, NeedGc> {
        #[cfg(debug_assertions)]
        {
            assert!(!self.stopped, "robigo: FillList::put after the stop: the pass did not end at the stop");
            assert_eq!(i, self.expect, "robigo: FillList::put of element {i}, expected {}", self.expect);
            self.expect += 1;
        }
        let v = match v {
            Ok(v) => v,
            Err(e) => return Err(self.refused(i, e)),
        };
        match &mut self.into {
            IntoList::Fresh(vec) => vec.set(i, v),
            IntoList::Rooted(x) => {
                assert!(i < self.n, "robigo: element {i} of {}", self.n);
                let s = v.sexp();
                self.cx.iso().check_owns(s);
                // (d) SAFETY: element `i < n` of the live VECSXP/EXPRSXP read
                // from the result's root this pass, and a live object of this
                // heap; `set_elt` maintains the barrier and reference counts.
                unsafe { x.set_elt(i, s) };
            }
        }
        if self.poll.load(Ordering::Relaxed) != 0 && i + 1 < self.n {
            self.next = i + 1;
            #[cfg(debug_assertions)]
            {
                self.stopped = true;
            }
            return Ok(true);
        }
        Ok(false)
    }

    #[cold]
    fn refused(&mut self, i: usize, e: NeedGc) -> NeedGc {
        self.next = i;
        e
    }

    #[inline(always)]
    fn check_done(&self, pass: &FillPass<'_>) {
        #[cfg(debug_assertions)]
        if matches!(pass, Ok(Ok(()))) && self.next >= self.n {
            assert_eq!(self.expect, self.n, "robigo: a fill_list pass returned with elements unmade");
        }
        let _ = pass;
    }
}

/// Where a pass of [`Cx::fill_pairlist`]'s loop appends cells front to back.
///
/// Holds H's four conditions for pausing mid-build (including to raise a
/// per-element tag warning whose calling handler may allocate, move objects,
/// error, or re-enter):
/// 1. The half-built chain is reachable only from the fill's own `head` and
///    `tail` roots; `out` is unpublished until the fill finishes.
/// 2. The caller's inputs are reached through its own roots and re-read on
///    every pass after a serve or warning handler.
/// 3. The fill is unwind-safe: an error or warning handler that jumps out
///    drops the scope's roots and leaves `out` untouched.
/// 4. The fill keeps no state outside its call (re-entrant).
///
/// A view kept from one pass for the next does not compile:
///
/// ```compile_fail
/// # use robigo_rtcore::{OwnedIsolate, Ref};
/// # let mut iso = OwnedIsolate::new();
/// # let cx = iso.cx();
/// cx.scope(|cx, s| {
///     let out = s.out(cx);
///     let mut kept: Option<Ref<'_>> = None;
///     cx.fill_pairlist(out, 1, |cx, fill| {
///         let v = kept.unwrap_or_else(|| cx.nil());
///         kept = Some(v);
///         fill.put(0, Ok((v, None, None))).map(|_| Ok(()))
///     })
/// })
/// .unwrap();
/// ```
pub struct FillPairlist<'a> {
    head: Option<Sexp>,
    tail: Option<Sexp>,
    start: usize,
    /// Number of cells already appended to the chain (resumption point on
    /// stop or refusal).
    done: usize,
    n: usize,
    warning: Option<String>,
    poll_stopped: bool,
    poll: &'a AtomicU32,
    cx: &'a Cx,
    #[cfg(debug_assertions)]
    stopped: bool,
}

impl<'a> FillPairlist<'a> {
    /// The element this pass starts at: the loop makes elements
    /// `start()..len`.
    #[inline(always)]
    pub fn start(&self) -> usize {
        self.start
    }

    /// Append element `i` (each in turn from `start()`): `(value, tag,
    /// warning)`. The cons cell is allocated and linked onto the tail through
    /// the write barrier (`set_cdr`). If `elt` or the cell allocation is
    /// refused (`Err(NeedGc)`), neither the cell nor its warning is kept and
    /// the fill retries from `i`. If a warning is present or a poll bit is
    /// set, returns `Ok(true)` so the pass ends: the fill roots `head` and
    /// `tail`, raises the warning (if any) once at `i`, serves the safepoint
    /// (if due), and resumes at `i + 1`.
    #[inline(always)]
    pub fn put(
        &mut self,
        i: usize,
        elt: Result<(Ref<'a>, Option<Sym<'a>>, Option<String>), NeedGc>,
    ) -> Result<bool, NeedGc> {
        #[cfg(debug_assertions)]
        {
            assert!(!self.stopped, "robigo: FillPairlist::put after the stop: the pass did not end at the stop");
            assert_eq!(i, self.done, "robigo: FillPairlist::put of element {i}, expected {}", self.done);
        }
        let (val, tag, warning) = elt?;
        let cell = match tag {
            Some(t) => self.cx.cons_tagged(val, self.cx.nil(), t)?,
            None => self.cx.cons(val, self.cx.nil())?,
        }
        .sexp();
        match self.tail {
            None => {
                self.head = Some(cell);
                self.tail = Some(cell);
            }
            Some(tail) => {
                // SAFETY: `tail` is a live cons cell of this isolate (either
                // made in this pass or read from `tail_root` at pass entry),
                // and `cell` is a fresh cons cell of this isolate; `set_cdr`
                // maintains the write barrier and reference counts.
                unsafe { tail.set_cdr(cell) };
                self.tail = Some(cell);
            }
        }
        self.done = i + 1;
        let poll_due = self.poll.load(Ordering::Relaxed) != 0 && i + 1 < self.n;
        if warning.is_some() || poll_due {
            self.warning = warning;
            self.poll_stopped = poll_due;
            #[cfg(debug_assertions)]
            {
                self.stopped = true;
            }
            return Ok(true);
        }
        Ok(false)
    }

    #[inline(always)]
    fn check_done(&self, pass: &FillPass<'_>) {
        #[cfg(debug_assertions)]
        if matches!(pass, Ok(Ok(()))) && !self.stopped {
            assert_eq!(self.done, self.n, "robigo: a fill_pairlist pass returned with elements unmade");
        }
        let _ = pass;
    }
}

/// Where a pass stopped: the next element to make, and the refusal that
/// stopped it, if one did (else a poll bit).
type Stop = Option<(usize, Option<NeedGc>)>;

impl Cx {
    /// A character vector of `n` elements, made by the loop `f` (elements
    /// `fill.start()..n`, each stored with [`Fill::put`]), written to `out`,
    /// with GNU R's collections between elements (see the module's
    /// promises). The first pass is one `&Cx` interval, as
    /// [`Cx::build_with`]'s: when nothing is due, nothing else runs.
    pub fn fill_strings<'r>(
        &mut self,
        out: Out<'_>,
        n: usize,
        mut f: impl for<'a> FnMut(&'a Cx, &mut Fill<'a>) -> FillPass<'r>,
    ) -> RResult<()> {
        // (a) the result's slot, read again after every serve
        let res = out.root();
        let stop: Stop = self.build_with(out, |cx| {
            // (c) "" in every element until it is filled
            let v = cx.alloc_vector::<Char<'_>>(n)?;
            let mut fill = Fill {
                into: Into::Fresh(v),
                start: 0,
                next: n,
                n,
                poll: &cx.iso().poll,
                cx,
                #[cfg(debug_assertions)]
                expect: 0,
                #[cfg(debug_assertions)]
                stopped: false,
            };
            let pass = f(cx, &mut fill);
            fill.check_done(&pass);
            let next = fill.next;
            let Into::Fresh(v) = fill.into else { unreachable!() };
            let v = v.finish().into();
            Ok(match pass {
                Ok(Ok(())) if next >= n => Built::Done(v, None),
                // stopped by `put`: the partial result goes to `out`
                Ok(Ok(())) => Built::Done(v, Some((next, None))),
                Ok(Err(d)) => Built::fail(d),
                // (a refusal `put` did not see, before any element: the
                // pass again from its start)
                Err(e) => Built::Done(v, Some((if next >= n { 0 } else { next }, Some(e)))),
            })
        })?;
        match stop {
            None => Ok(()),
            Some(s) => self.fill_strings_resume(res, n, &mut f, s),
        }
    }

    /// The passes after the first: serve what stopped the last pass, then
    /// run the loop on from where it stopped, through the result's root.
    #[cold]
    #[inline(never)]
    fn fill_strings_resume<'r>(
        &mut self,
        res: Root<'_>,
        n: usize,
        f: &mut impl for<'a> FnMut(&'a Cx, &mut Fill<'a>) -> FillPass<'r>,
        mut stop: (usize, Option<NeedGc>),
    ) -> RResult<()> {
        // the refusal served last, while no element has been made since
        let mut refused: Option<NeedGc> = None;
        loop {
            let (start, e) = stop;
            match e {
                Some(e) if refused == Some(e) => return Err(self.refused_again(e)),
                Some(e) => {
                    self.need_gc(e)?;
                    refused = Some(e);
                }
                None => self.safepoint(Safepoint::Loop)?,
            }
            let (pass, next) = {
                let cx = &*self;
                // (a) the result read again through its root
                let x = res.bind(cx);
                debug_assert!(x.sexptype() == STRSXP && x.len() == Some(n));
                let mut fill = Fill {
                    into: Into::Rooted(x.sexp()),
                    start,
                    next: n,
                    n,
                    poll: &cx.iso().poll,
                    cx,
                    #[cfg(debug_assertions)]
                    expect: start,
                    #[cfg(debug_assertions)]
                    stopped: false,
                };
                let pass = f(cx, &mut fill);
                fill.check_done(&pass);
                let next = match pass {
                    // (a refusal `put` did not see: the pass again)
                    Err(_) if fill.next >= n => start,
                    _ => fill.next,
                };
                (pass, next)
            };
            if next > start {
                refused = None;
            }
            match pass {
                Ok(Ok(())) if next >= n => return Ok(()),
                Ok(Ok(())) => stop = (next, None),
                Ok(Err(d)) => return Err(self.raise(d)),
                Err(e) => stop = (next, Some(e)),
            }
        }
    }

    /// A list (`ListKind::List`) or expression vector (`ListKind::Expr`) of
    /// `n` elements, made by the loop `f` (elements `fill.start()..n`, each
    /// completed whole and stored with [`FillList::put`]), written to `out`,
    /// with GNU R's collections between elements.
    pub fn fill_list<'r>(
        &mut self,
        out: Out<'_>,
        kind: ListKind,
        n: usize,
        mut f: impl for<'a> FnMut(&'a Cx, &mut FillList<'a>) -> FillPass<'r>,
    ) -> RResult<()> {
        let res = out.root();
        let stop: Stop = self.build_with(out, |cx| {
            // (c) R_NilValue (immortal) in every element until it is filled
            let v = match kind {
                ListKind::List => cx.alloc_vector::<Ref<'_>>(n)?,
                ListKind::Expr => cx.alloc_expression(n)?,
            };
            let mut fill = FillList {
                into: IntoList::Fresh(v),
                start: 0,
                next: n,
                n,
                poll: &cx.iso().poll,
                cx,
                #[cfg(debug_assertions)]
                expect: 0,
                #[cfg(debug_assertions)]
                stopped: false,
            };
            let pass = f(cx, &mut fill);
            fill.check_done(&pass);
            let next = fill.next;
            let IntoList::Fresh(v) = fill.into else { unreachable!() };
            let v = v.finish().into();
            Ok(match pass {
                Ok(Ok(())) if next >= n => Built::Done(v, None),
                Ok(Ok(())) => Built::Done(v, Some((next, None))),
                Ok(Err(d)) => Built::fail(d),
                Err(e) => Built::Done(v, Some((if next >= n { 0 } else { next }, Some(e)))),
            })
        })?;
        match stop {
            None => Ok(()),
            Some(s) => self.fill_list_resume(res, kind, n, &mut f, s),
        }
    }

    /// The passes after the first for [`Cx::fill_list`].
    #[cold]
    #[inline(never)]
    fn fill_list_resume<'r>(
        &mut self,
        res: Root<'_>,
        kind: ListKind,
        n: usize,
        f: &mut impl for<'a> FnMut(&'a Cx, &mut FillList<'a>) -> FillPass<'r>,
        mut stop: (usize, Option<NeedGc>),
    ) -> RResult<()> {
        let mut refused: Option<NeedGc> = None;
        loop {
            let (start, e) = stop;
            match e {
                Some(e) if refused == Some(e) => return Err(self.refused_again(e)),
                Some(e) => {
                    self.need_gc(e)?;
                    refused = Some(e);
                }
                None => self.safepoint(Safepoint::Loop)?,
            }
            let (pass, next) = {
                let cx = &*self;
                let x = res.bind(cx);
                debug_assert!(x.sexptype() == kind.sexptype() && x.len() == Some(n));
                let mut fill = FillList {
                    into: IntoList::Rooted(x.sexp()),
                    start,
                    next: n,
                    n,
                    poll: &cx.iso().poll,
                    cx,
                    #[cfg(debug_assertions)]
                    expect: start,
                    #[cfg(debug_assertions)]
                    stopped: false,
                };
                let pass = f(cx, &mut fill);
                fill.check_done(&pass);
                let next = match pass {
                    Err(_) if fill.next >= n => start,
                    _ => fill.next,
                };
                (pass, next)
            };
            if next > start {
                refused = None;
            }
            match pass {
                Ok(Ok(())) if next >= n => return Ok(()),
                Ok(Ok(())) => stop = (next, None),
                Ok(Err(d)) => return Err(self.raise(d)),
                Err(e) => stop = (next, Some(e)),
            }
        }
    }

    /// A pairlist of `n` cells built front to back by `f` (elements
    /// `fill.start()..n`, each appended with [`FillPairlist::put`]), with
    /// collections and per-element warnings served between cells. The
    /// half-built chain is kept only in this call's scope roots and `out` is
    /// written once all `n` cells (and any final warning) succeed.
    pub fn fill_pairlist<'r>(
        &mut self,
        out: Out<'_>,
        n: usize,
        mut f: impl for<'a> FnMut(&'a Cx, &mut FillPairlist<'a>) -> FillPass<'r>,
    ) -> RResult<()> {
        if n == 0 {
            out.set(self, self.nil());
            return Ok(());
        }
        self.scope(|cx, s| {
            let head_root = s.root(cx, cx.nil());
            let tail_root = s.root(cx, cx.nil());
            let mut start = 0usize;
            let mut refused: Option<NeedGc> = None;
            #[cfg(feature = "retry-torture")]
            let mut first = cx.retry_torture();
            loop {
                let (pass, new_head, new_tail, done, warning, poll_stopped) = {
                    #[cfg(feature = "retry-torture")]
                    let _refuse = cx.refuse_once(&mut first);
                    let cx = &*cx;
                    let nil = cx.nil().sexp();
                    let h = head_root.bind(cx).sexp();
                    let t = tail_root.bind(cx).sexp();
                    let mut fill = FillPairlist {
                        head: (h != nil).then_some(h),
                        tail: (t != nil).then_some(t),
                        start,
                        done: start,
                        n,
                        warning: None,
                        poll_stopped: false,
                        poll: &cx.iso().poll,
                        cx,
                        #[cfg(debug_assertions)]
                        stopped: false,
                    };
                    let pass = f(cx, &mut fill);
                    fill.check_done(&pass);
                    (pass, fill.head, fill.tail, fill.done, fill.warning, fill.poll_stopped)
                };
                // Fast path: finished all `n` cells with no warning and no stop.
                if matches!(pass, Ok(Ok(()))) && done >= n && warning.is_none() {
                    let h = new_head.expect("robigo: non-empty pairlist fill ended without head");
                    // SAFETY: `h` is a live cons cell of this isolate and
                    // nothing has collected since the pass returned.
                    out.set(cx, unsafe { Ref::from_sexp(h, cx) });
                    return Ok(());
                }
                // Root the partial chain before raising a warning, collecting,
                // or serving a safepoint (any of which may move objects).
                if let Some(h) = new_head {
                    let o = head_root.reset(cx);
                    // SAFETY: `h` is a live cons cell of this isolate and
                    // nothing has collected since the pass returned.
                    o.set(cx, unsafe { Ref::from_sexp(h, cx) });
                }
                if let Some(t) = new_tail {
                    let o = tail_root.reset(cx);
                    // SAFETY: as above.
                    o.set(cx, unsafe { Ref::from_sexp(t, cx) });
                }
                if done > start {
                    refused = None;
                }
                match pass {
                    Ok(Ok(())) => {
                        if let Some(msg) = warning {
                            cx.warning(&msg)?;
                        }
                        if done >= n {
                            out.set(cx, head_root.bind(cx));
                            return Ok(());
                        }
                        if poll_stopped || cx.gc_due() {
                            cx.safepoint(Safepoint::Loop)?;
                        }
                        start = done;
                    }
                    Ok(Err(d)) => return Err(cx.raise(d)),
                    Err(e) if refused == Some(e) => return Err(cx.refused_again(e)),
                    Err(e) => {
                        cx.need_gc(e)?;
                        refused = Some(e);
                        start = done;
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{Diag, Hooks, OwnedIsolate};
    use robigo_heap::types::NILSXP;
    use std::cell::RefCell;

    #[test]
    fn fill_pairlist_builds_front_to_back_and_handles_empty() {
        let mut iso = OwnedIsolate::new();
        let cx = iso.cx();
        cx.scope(|cx, s| {
            let empty = s.out(cx);
            let er = empty.root();
            cx.fill_pairlist(empty, 0, |_cx, _fill| unreachable!()).unwrap();
            assert_eq!(er.bind(cx).sexptype(), NILSXP);

            let out = s.out(cx);
            let r = out.root();
            cx.fill_pairlist(out, 3, |cx, fill| {
                for i in fill.start()..3 {
                    let v = cx.alloc_copy(&[i as i32 + 10])?.finish().into();
                    let tag = if i == 1 { None } else { Some(cx.install(format!("k{i}").as_bytes())?) };
                    if fill.put(i, Ok((v, tag, None)))? {
                        return Ok(Ok(()));
                    }
                }
                Ok(Ok(()))
            })
            .unwrap();

            let mut cur = r.bind(cx).as_cons();
            let mut got = Vec::new();
            while let Some(c) = cur {
                let n = c.car().as_scalar::<i32>().unwrap();
                let t = c.tag().as_sym().map(|sym| sym.name_bytes().to_vec());
                got.push((t, n));
                cur = c.cdr().as_cons();
            }
            assert_eq!(got, vec![(Some(b"k0".to_vec()), 10), (None, 11), (Some(b"k2".to_vec()), 12)]);
        });
    }

    #[test]
    fn fill_pairlist_warning_stops_and_unwind_leaves_out_unpublished() {
        thread_local! {
            static WARNS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
            static FAIL_SECOND: RefCell<bool> = const { RefCell::new(false) };
        }
        let mut iso = OwnedIsolate::new();
        let cx = iso.cx();
        cx.install_hooks(Hooks {
            warning: |cx, _call, msg| {
                WARNS.with(|w| w.borrow_mut().push(String::from_utf8_lossy(msg).into_owned()));
                if FAIL_SECOND.with(|f| *f.borrow()) && WARNS.with(|w| w.borrow().len() >= 2) {
                    return Err(cx.error("handler boom"));
                }
                Ok(())
            },
            ..Hooks::default()
        });

        // Two warnings (one mid-chain, one on the final element): both fire in order.
        WARNS.with(|w| w.borrow_mut().clear());
        FAIL_SECOND.with(|f| *f.borrow_mut() = false);
        cx.scope(|cx, s| {
            let out = s.out(cx);
            let r = out.root();
            cx.fill_pairlist(out, 3, |cx, fill| {
                for i in fill.start()..3 {
                    let v = cx.alloc_copy(&[i as i32])?.finish().into();
                    let w = (i == 0 || i == 2).then(|| format!("warn {i}"));
                    if fill.put(i, Ok((v, None, w)))? {
                        return Ok(Ok(()));
                    }
                }
                Ok(Ok(()))
            })
            .unwrap();
            let mut vals = Vec::new();
            let mut cur = r.bind(cx).as_cons();
            while let Some(c) = cur {
                vals.push(c.car().as_scalar::<i32>().unwrap());
                cur = c.cdr().as_cons();
            }
            assert_eq!(vals, vec![0, 1, 2]);
        });
        assert_eq!(WARNS.with(|w| w.borrow().clone()), vec!["warn 0", "warn 2"]);

        // Handler error on second warning unwinds cleanly with `out` still empty (NULL).
        WARNS.with(|w| w.borrow_mut().clear());
        FAIL_SECOND.with(|f| *f.borrow_mut() = true);
        cx.scope(|cx, s| {
            let out = s.out(cx);
            let r = out.root();
            let res = cx.fill_pairlist(out, 3, |cx, fill| {
                for i in fill.start()..3 {
                    let v = cx.alloc_copy(&[i as i32])?.finish().into();
                    let w = Some(format!("warn {i}"));
                    if fill.put(i, Ok((v, None, w)))? {
                        return Ok(Ok(()));
                    }
                }
                Ok(Ok(()))
            });
            let err = res.unwrap_err();
            assert_eq!(r.bind(cx).sexptype(), NILSXP);
            let _ = cx.take_exit(err);
        });

        // Diagnostic error mid-fill leaves `out` untouched.
        cx.scope(|cx, s| {
            let out = s.out(cx);
            let r = out.root();
            let res = cx.fill_pairlist(out, 2, |cx, fill| {
                for i in fill.start()..2 {
                    if i == 1 {
                        return Ok(Err(Diag::error("bad bytes")));
                    }
                    let v = cx.alloc_copy(&[i as i32])?.finish().into();
                    if fill.put(i, Ok((v, None, None)))? {
                        return Ok(Ok(()));
                    }
                }
                Ok(Ok(()))
            });
            let err = res.unwrap_err();
            assert_eq!(r.bind(cx).sexptype(), NILSXP);
            let _ = cx.take_exit(err);
        });
    }
}
