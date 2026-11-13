//! Errors and exits (docs/g3.md §6.5): `RResult`'s zero-sized error token,
//! the pending-exit stack whose top record it stands for, and `NeedGc`, the
//! allocator's own failure (§3 item 5), which is not an `RError`.
//!
//! This module declares the types and the minimum that makes a token: an
//! exit is recorded ([`Cx::exit`]), inspected and resolved. Parking a record
//! in the context being left while cleanup code runs, superseding exits and
//! unwind continuations owning their record are C2's (`context.c:198-244,
//! 298-319, 977-993`).

#![allow(unsafe_code)]

use crate::isolate::Cx;
use crate::root::{Out, Root};
use robigo_heap::{NeedGc, Sexp, Trace};

/// An exit is in progress: an R error, a `return`/`break`/`next` that
/// crosses frames, a restart, an interrupt. Its record (kind, target,
/// value, condition, call) is the top of the isolate's pending-exit stack,
/// a precise root, so the token itself carries nothing.
///
/// Only [`Cx::exit`] makes one, so every `Err` has its record. A token is
/// neither `Copy` nor `Clone`: each stands for exactly one record, which
/// [`Cx::resolve_exit`] consumes.
#[must_use = "an RError stands for a pending exit: return it, or resolve it with Cx::resolve_exit"]
#[derive(Debug, PartialEq, Eq)]
pub struct RError {
    _record: (),
}

/// The result of anything that may exit: zero-sized error, so
/// `RResult<Ref>` is one word (asserted in `lib.rs`).
pub type RResult<T> = Result<T, RError>;

/// What kind of exit a record is. C2 refines these into GNU R's jump
/// records (`R_jumpctxt` targets and masks, restarts, `R_ContinueUnwind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ExitKind {
    /// An R error (`stop`, `errorcall`, a C `Rf_error`).
    Error,
    /// A jump to a context: `return`, `break` and `next` across frames,
    /// the condition system's jumps to a handler's frame.
    Jump,
    /// `invokeRestart` to a restart's frame.
    Restart,
    /// A user interrupt (`onintr`).
    Interrupt,
}

/// What a pending exit is, as [`Cx::exit_info`] reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitInfo {
    pub kind: ExitKind,
    /// GNU R's jump mask (`CTXT_*`), 0 for none.
    pub mask: i32,
    /// The target context's address (C2's `RCNTXT`), 0 for the top level.
    pub target: usize,
}

/// An exit to record, with its objects in roots (so that nothing in it can
/// be collected between building it and recording it).
#[derive(Clone, Copy, Debug)]
pub struct Exit<'s> {
    pub info: ExitInfo,
    /// The value the jump carries (`R_ReturnedValue`), or the error message.
    pub value: Option<Root<'s>>,
    /// The condition object, once S1 builds them.
    pub condition: Option<Root<'s>>,
    /// The call the exit reports (`errorcall`'s call).
    pub call: Option<Root<'s>>,
}

impl<'s> Exit<'s> {
    /// An exit of `kind` to the top level, carrying nothing.
    pub fn new(kind: ExitKind) -> Exit<'s> {
        Exit { info: ExitInfo { kind, mask: 0, target: 0 }, value: None, condition: None, call: None }
    }
}

/// One record of the pending-exit stack. Its objects are traced
/// ([`Cx::for_each_root`]); a field with no object is R_NilValue.
#[derive(Clone, Copy, Debug, Trace)]
pub(crate) struct ExitRecord {
    #[trace(skip = "kind, mask and a context address: no objects")]
    pub(crate) info: ExitInfo,
    pub(crate) value: Sexp,
    pub(crate) condition: Sexp,
    pub(crate) call: Sexp,
}

/// The isolate's pending-exit stack (docs/g3.md §6.5): innermost exit on
/// top. A precise root.
#[derive(Default, Trace)]
pub(crate) struct PendingExits {
    pub(crate) records: Vec<ExitRecord>,
}

impl Cx {
    /// Record `exit` on top of the pending-exit stack and return the token
    /// that stands for it. Panics if one of its roots belongs to another
    /// isolate.
    pub fn exit(&mut self, exit: Exit<'_>) -> RError {
        let nil = self.iso().nil;
        let read = |r: Option<Root<'_>>| r.map_or(nil, |r| r.bind(self).sexp());
        let rec = ExitRecord {
            info: exit.info,
            value: read(exit.value),
            condition: read(exit.condition),
            call: read(exit.call),
        };
        self.iso().exit_push(rec);
        RError { _record: () }
    }

    /// Record an error with message `msg` reported against `call` (GNU R's
    /// `errorcall`; `None` is `errorcall(R_NilValue, ...)`, no call). The
    /// message is the record's value, a character vector of one element.
    /// S1 turns the record into GNU R's condition and its reporting
    /// (`verrorcall_dflt`); `error()`, which reports the context's call,
    /// arrives with C2's contexts. The text is the caller's, verbatim.
    pub fn error_call(&mut self, call: Option<Root<'_>>, msg: &str) -> RError {
        let cx = &*self;
        let iso = cx.iso();
        // A message vector that cannot be allocated leaves the value
        // R_NilValue: the error is still recorded.
        // a C string's message ends at a NUL
        let msg = msg.split('\0').next().unwrap_or_default();
        let value = cx
            .mk_char(msg.as_bytes(), crate::Encoding::Utf8)
            .ok()
            .and_then(|c| {
                let mut v = cx.alloc_vector::<crate::Char<'_>>(1).ok()?;
                v.set(0, c);
                Some(v.finish().as_ref().sexp())
            })
            .unwrap_or(iso.nil);
        let call = call.map_or(iso.nil, |c| c.bind(cx).sexp());
        let rec = ExitRecord {
            info: ExitInfo { kind: ExitKind::Error, mask: 0, target: 0 },
            value,
            condition: iso.nil,
            call,
        };
        iso.exit_push(rec);
        RError { _record: () }
    }

    /// The answer of the first `&mut Cx` frame to a failed `&Cx`
    /// allocation (docs/g3.md §3 item 5): collect (a `Retry` safepoint, with
    /// the isolate's precise roots), then check GNU R's limits for the
    /// request. `Ok(())`: retry exactly that allocation. `Err`: GNU R's
    /// error ("vector memory limit ... reached", "cons memory ...", "cannot
    /// allocate vector of size ..."), recorded with no call. [`Cx::build`]
    /// is the usual way to use it.
    pub fn need_gc(&mut self, e: NeedGc) -> RResult<()> {
        let iso = self.iso();
        // SAFETY: `&mut Cx` ends every `Ref`, view and slice of this
        // capability, so everything Rust code still needs is in a root of
        // this isolate (the minting rule, lib.rs). Every root edge is null,
        // immortal or a live object of this heap (every write into a root is
        // owner-checked). No other reference into the core or the roots is
        // live, and the collection reaches nothing of ours (finalizers are
        // only queued).
        let r = unsafe { iso.core().make_room(e, iso.roots_mut()) };
        r.map_err(|a| self.error_call(None, &a.message()))
    }

    /// The innermost pending exit: the top of the pending-exit stack. `e`
    /// is the evidence that one is pending, not a pointer to its record (the
    /// token is zero-sized): it is `e`'s record while no exit raised after
    /// `e` is still pending (C2 parks those in the context being left).
    pub fn exit_info(&self, e: &RError) -> ExitInfo {
        let _ = e; // (zero-sized: it names no record)
        let iso = self.iso();
        iso.exit_count()
            .checked_sub(1)
            .and_then(|i| iso.exit_at(i))
            .expect("robigo: an RError without its pending-exit record")
            .info
    }

    /// The call the innermost pending exit reports (`errorcall`'s call;
    /// R_NilValue for none): `e`'s, under the rule of [`Cx::exit_info`].
    pub fn exit_call<'g>(&'g self, e: &RError) -> crate::Ref<'g> {
        let _ = e; // (zero-sized: it names no record)
        let iso = self.iso();
        let rec = iso
            .exit_count()
            .checked_sub(1)
            .and_then(|i| iso.exit_at(i))
            .expect("robigo: an RError without its pending-exit record");
        // SAFETY: the record is a precise root while `e` is pending, and
        // nothing can resolve it during this `&Cx` interval.
        unsafe { crate::Ref::from_sexp(rec.call, self) }
    }

    /// The innermost pending exit has been handled: pop the top of the
    /// pending-exit stack (`e`'s record, under the rule of
    /// [`Cx::exit_info`]), writing the value it carried into `value` if
    /// given, and consume `e`.
    pub fn resolve_exit(&mut self, e: RError, value: Option<Out<'_>>) -> ExitInfo {
        let RError { _record: () } = e;
        let rec = self.iso().exit_pop().expect("robigo: an RError without its pending-exit record");
        if let Some(out) = value {
            // SAFETY: the record's value was a precise root until the pop
            // above, and nothing collected since: this is one `&mut Cx`
            // interval with no safepoint.
            out.set(self, unsafe { crate::Ref::from_sexp(rec.value, self) });
        }
        rec.info
    }

    /// How many exits are pending (nested exits: an `on.exit` that errors
    /// while an error unwinds, C2).
    pub fn pending_exits(&self) -> usize {
        self.iso().exit_count()
    }
}
