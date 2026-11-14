//! What a `&Cx` phase ends with (docs/g3.md §2.4; the E6 review, change 1):
//! [`Cx::build_with`] runs a closure that reads its inputs and builds a
//! result under `&Cx`, and the closure says how it ended. [`Built::Done`]
//! carries the result and plain data about it (warning flags, counts),
//! which the successful attempt returns to the caller; [`Built::Fail`]
//! carries an error found while reading, a [`Diag`], raised once the read
//! interval has ended (an exit takes `&mut Cx`). So a builtin needs no flag
//! captured from the closure and reset on every attempt, no sentinel
//! result, and no staging buffer to find its error before it allocates; and
//! its warnings, signalled from the returned data, stay in GNU R's order
//! after the computation.

use crate::exit::{RError, RResult};
use crate::handle::Ref;
use crate::isolate::Cx;
use crate::root::{Out, Root};
use robigo_heap::{AllocError, NeedGc};

/// An error found under `&Cx`, as plain data, to raise once the read
/// interval has ended ([`Cx::raise`]): GNU R's `error(msg)` or
/// `errorcall(call, msg)`, with the message verbatim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diag<'r> {
    call: DiagCall<'r>,
    msg: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DiagCall<'r> {
    /// `error()`: the context's call.
    Context,
    /// `errorcall(call, ...)`; None is `errorcall(R_NilValue, ...)`.
    Call(Option<Root<'r>>),
}

impl<'r> Diag<'r> {
    /// GNU R's `error(msg)`: reported against the context's call
    /// ([`Cx::error`]).
    pub fn error(msg: impl Into<String>) -> Diag<'r> {
        Diag { call: DiagCall::Context, msg: msg.into() }
    }

    /// GNU R's `errorcall(call, msg)`; `None` is `errorcall(R_NilValue,
    /// msg)`, no call ([`Cx::error_call`]).
    pub fn error_call(call: Option<Root<'r>>, msg: impl Into<String>) -> Diag<'r> {
        Diag { call: DiagCall::Call(call), msg: msg.into() }
    }

    /// The message.
    pub fn message(&self) -> &str {
        &self.msg
    }
}

/// How one attempt of [`Cx::build_with`] ends.
#[derive(Debug)]
pub enum Built<'g, 'r, M> {
    /// The result, which `build_with` writes to its `out`, and plain data
    /// about it, which `build_with` returns.
    Done(Ref<'g>, M),
    /// An error found while reading the inputs: `build_with` raises it once
    /// the read interval has ended, and writes nothing to `out`.
    Fail(Diag<'r>),
}

impl<'g, 'r> Built<'g, 'r, ()> {
    /// The result, with no data about it.
    pub fn done(r: impl Into<Ref<'g>>) -> Self {
        Built::Done(r.into(), ())
    }
}

impl<'g, 'r, M> Built<'g, 'r, M> {
    /// The same outcome, a result carrying `meta` instead.
    pub fn with<N>(self, meta: N) -> Built<'g, 'r, N> {
        match self {
            Built::Done(r, _) => Built::Done(r, meta),
            Built::Fail(d) => Built::Fail(d),
        }
    }

    /// An error to raise after the read interval.
    pub fn fail(d: Diag<'r>) -> Self {
        Built::Fail(d)
    }
}

impl Cx {
    /// Raise `d`: [`Cx::error`] or [`Cx::error_call`].
    pub fn raise(&mut self, d: Diag<'_>) -> RError {
        match d.call {
            DiagCall::Context => self.error(&d.msg),
            DiagCall::Call(c) => self.error_call(c, &d.msg),
        }
    }

    /// Build a result under `&Cx` and write it to `out`, or raise the error
    /// the attempt found: `f` reads its inputs (through roots or `Args`)
    /// and returns [`Built::Done`] with the result and any plain data the
    /// caller needs afterwards (warnings to signal, in GNU R's order, once
    /// this returns), or [`Built::Fail`].
    ///
    /// When an allocation in `f` is refused, [`Cx::need_gc`] collects and
    /// `f` runs again from the start; the same request refused again after
    /// that is GNU R's "cannot allocate vector of size ...". Re-running `f`
    /// is always allowed: under `&Cx` it can commit no observable effect,
    /// and what it returns comes from the attempt that succeeded, so it
    /// needs no state reset between attempts. (A host effect inside `f`, a
    /// counter or a file write, would repeat: keep those outside.)
    /// A raw allocation step under `&Cx` (a node, a symbol: what is not a
    /// built result), retried as [`Cx::build_with`] retries: after a
    /// refusal [`Cx::need_gc`] collects and `f` runs again; the same request
    /// refused again is GNU R's "cannot allocate vector of size ...", so a
    /// refusal that a collection cannot answer ends instead of looping.
    pub fn retry_alloc<T>(&mut self, mut f: impl FnMut(&Cx) -> Result<T, NeedGc>) -> RResult<T> {
        let mut retried = None;
        loop {
            match f(self) {
                Ok(t) => return Ok(t),
                Err(e) if retried == Some(e) => {
                    let vecrecs = e.length().saturating_mul(robigo_heap::elt_size(e.sexptype())).div_ceil(8);
                    return Err(self.error_call(None, &AllocError::CannotAllocate { vecrecs }.message()));
                }
                Err(e) => {
                    self.need_gc(e)?;
                    retried = Some(e);
                }
            }
        }
    }

    pub fn build_with<'r, M>(
        &mut self,
        out: Out<'_>,
        mut f: impl for<'g> FnMut(&'g Cx) -> Result<Built<'g, 'r, M>, NeedGc>,
    ) -> RResult<M> {
        let mut retried = None;
        loop {
            match f(self) {
                Ok(Built::Done(r, m)) => {
                    out.set(self, r);
                    return Ok(m);
                }
                Ok(Built::Fail(d)) => return Err(self.raise(d)),
                Err(e) if retried == Some(e) => {
                    let vecrecs = e.length().saturating_mul(robigo_heap::elt_size(e.sexptype())).div_ceil(8);
                    return Err(self.error_call(None, &AllocError::CannotAllocate { vecrecs }.message()));
                }
                Err(e) => {
                    self.need_gc(e)?;
                    retried = Some(e);
                }
            }
        }
    }
}
