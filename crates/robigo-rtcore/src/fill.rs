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
//!   barrier and the counts (the vector may be old by then); before the
//!   first serve the vector is fresh, unpublished, and filled as
//!   [`FreshVec::set`] fills one.

#![allow(unsafe_code)]

use crate::exit::RResult;
use crate::fresh::FreshVec;
use crate::handle::Char;
use crate::isolate::Cx;
use crate::outcome::{Built, Diag};
use crate::root::{Out, Root};
use robigo_heap::types::STRSXP;
use robigo_heap::{NeedGc, Safepoint, Sexp};
use std::sync::atomic::{AtomicU32, Ordering};

/// What a pass of the builtin's loop ends with: done or stopped by
/// [`Fill::put`] (`Ok(Ok(()))`), an error to raise (the partial result is
/// dropped), or the allocation the heap refused for the element given to
/// [`Fill::put`] (the fill collects and runs the loop again from that
/// element; refused again, it is GNU R's error for the cause).
pub type FillPass = Result<Result<(), Diag<'static>>, NeedGc>;

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
    fn check_done(&self, pass: &FillPass) {
        #[cfg(debug_assertions)]
        if matches!(pass, Ok(Ok(()))) && self.next >= self.n {
            assert_eq!(self.expect, self.n, "robigo: a fill pass returned with elements unmade");
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
    pub fn fill_strings(
        &mut self,
        out: Out<'_>,
        n: usize,
        mut f: impl for<'a> FnMut(&'a Cx, &mut Fill<'a>) -> FillPass,
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
    fn fill_strings_resume(
        &mut self,
        res: Root<'_>,
        n: usize,
        f: &mut impl for<'a> FnMut(&'a Cx, &mut Fill<'a>) -> FillPass,
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
}
