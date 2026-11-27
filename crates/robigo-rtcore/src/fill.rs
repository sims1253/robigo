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
use crate::handle::{Char, Ref};
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
///   and, if due, ends the pass with `put(i, Err(NeedGc::Bytes))`.
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
}
