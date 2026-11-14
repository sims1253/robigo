//! Errors and exits (docs/g3.md §6.5): `RResult`'s zero-sized error token,
//! the pending-exit stack whose top record it stands for, and `NeedGc`, the
//! allocator's own failure (§3 item 5), which is not an `RError`.
//!
//! This module declares the types and the minimum that makes a token: an
//! exit is recorded ([`Cx::exit`]), inspected and resolved. How an exit
//! travels the context chain (legs, parking a record in the context being
//! left while its exit code runs, superseding exits, unwind continuations
//! owning their record, `context.c:198-244, 298-319, 977-993`) is in
//! `context.rs` and `native.rs`.

#![allow(unsafe_code)]

use crate::handle::Ref;
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

impl RError {
    /// The token for the record just pushed (or made pending again).
    #[inline(always)]
    pub(crate) fn new() -> RError {
        RError { _record: () }
    }
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

pub(crate) fn kind_from_u8(k: u8) -> ExitKind {
    match k {
        0 => ExitKind::Error,
        1 => ExitKind::Jump,
        2 => ExitKind::Restart,
        _ => ExitKind::Interrupt,
    }
}

/// What a pending exit is, as [`Cx::exit_info`] reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitInfo {
    pub kind: ExitKind,
    /// GNU R's jump mask (`CTXT_*`), 0 for none.
    pub mask: i32,
    /// The target context's address ([`crate::ContextId::addr`]). Recording
    /// an exit with 0 targets `R_ToplevelContext` as it is then.
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
    /// The context this leg of the exit stops at (`first_jump_target`), 0
    /// before the leg starts (`context.rs`).
    #[trace(skip = "a context address: no object")]
    pub(crate) stop: usize,
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
        let mut info = exit.info;
        if info.target == 0 {
            info.target = self.ctxs().toplevel.get() as usize;
        }
        let rec = ExitRecord {
            info,
            value: read(exit.value),
            condition: read(exit.condition),
            call: read(exit.call),
            stop: 0,
        };
        self.iso().exit_push(rec);
        RError { _record: () }
    }

    /// Signal an error with message `msg` reported against `call`: GNU R's
    /// `errorcall` (`None` is `errorcall(R_NilValue, ...)`, no call),
    /// through the isolate's error hook ([`crate::Hooks::error`]): the
    /// condition system's handlers run here, where the error is raised,
    /// then its default handling reports it. The token stands for the exit
    /// that leaves. The text is the caller's, verbatim.
    pub fn error_call(&mut self, call: Option<Root<'_>>, msg: &str) -> RError {
        let call = call.map_or(self.iso().nil, |c| c.bind(self).sexp());
        self.error_call_sexp(call, msg)
    }

    /// Signal an error reported against the call of the current context,
    /// GNU R's `error()`: the innermost record as R sees the chain, or the
    /// one below it if that is a `CTXT_BUILTIN` record (a foreign call's).
    pub fn error(&mut self, msg: &str) -> RError {
        let call = self.error_context_call().sexp();
        self.error_call_sexp(call, msg)
    }

    /// The call GNU R's `error()` and `warning()` report: the innermost
    /// record as R sees the chain, or the one below it if that is a
    /// `CTXT_BUILTIN` record (a foreign call's); R_NilValue at the base.
    /// While byte code runs in the innermost record (`R_BCIntActive`), the
    /// expression it evaluates, as `R_getBCInterpreterExpression` maps it
    /// to what the AST interpreter would report: a symbol, a call of a
    /// closure wrapper (`.Internal`, `.Call`, ...) or of a primitive, or no
    /// known expression, is the innermost function's call.
    // R: errors.c:Rf_error, errors.c:getCurrentCall; eval.c:R_getBCInterpreterExpression
    pub fn error_context_call(&self) -> Ref<'_> {
        let g = self.global_context();
        if g.callflag() & crate::ctxt::BUILTIN != 0 {
            return g.next().map_or_else(|| self.nil(), |r| r.call());
        }
        let ctx = self.iso().contexts();
        if !ctx.bc_active.get() {
            return g.call();
        }
        // (a traced root of the context state: a constant of the running
        // code object, live)
        let e = self.raw_ref(ctx.bc_expr.get());
        // (an assignment call is not inflated: robigo-rt sets none)
        if e.sexptype() == robigo_heap::types::LANGSXP
            && !maybe_closure_wrapper(self, e)
            && !maybe_primitive_call(self, e)
        {
            return e;
        }
        let mut c = Some(g);
        while let Some(r) = c {
            if r.callflag() == crate::ctxt::TOPLEVEL {
                break;
            }
            if r.callflag() & crate::ctxt::FUNCTION != 0 {
                return r.call();
            }
            c = r.next();
        }
        if e == self.nil() { g.call() } else { e }
    }

    /// Run `f` as an activation of byte code (`bytecode`) or not: GNU R's
    /// `R_BCIntActive`, which `bcEval` sets and `eval` clears, saved and
    /// restored around it, with no current expression.
    #[inline]
    pub fn bc_activation<R>(&mut self, bytecode: bool, f: impl FnOnce(&mut Cx) -> R) -> R {
        let (active, expr) = {
            let c = self.iso().contexts();
            let saved = (c.bc_active.get(), c.bc_expr.get());
            c.bc_active.set(bytecode);
            c.bc_expr.set(self.iso().nil);
            saved
        };
        let r = f(self);
        let c = self.iso().contexts();
        c.bc_active.set(active);
        c.bc_expr.set(expr);
        r
    }

    /// The expression the byte code of the innermost activation evaluates
    /// (R_NilValue: none known), for [`Cx::error_context_call`].
    ///
    /// # Safety
    /// `e` is R_NilValue or a live object reachable from the running code
    /// object; it is traced while set.
    #[inline(always)]
    pub unsafe fn set_bc_expression(&self, e: Sexp) {
        self.iso().contexts().bc_expr.set(e);
    }

    /// [`Cx::error_call`] with the call as an object of a live record or
    /// root, read in this interval.
    pub(crate) fn error_call_sexp(&mut self, call: Sexp, msg: &str) -> RError {
        let hook = self.iso().hooks.get().error;
        let nil = self.iso().nil;
        self.scope(|cx, s| {
            let c = (call != nil).then(|| s.root(cx, cx.raw_ref(call)));
            hook(cx, c, msg)
        })
    }

    /// Record an error with message `msg` reported against `call` as an
    /// exit to the top level (`R_ToplevelContext`), running nothing: the
    /// record's value is the message, a character vector of one element
    /// (R_NilValue if it cannot be allocated), its call `call`. The error
    /// hook's default, and the condition system's last step (GNU R's
    /// `jump_to_top_ex` after `verrorcall_dflt` reported the error).
    pub fn record_error_call(&mut self, call: Option<Root<'_>>, msg: &str) -> RError {
        let cx = &*self;
        let iso = cx.iso();
        let call = call.map_or(iso.nil, |c| c.bind(cx).sexp());
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
        let rec = ExitRecord {
            info: ExitInfo { kind: ExitKind::Error, mask: 0, target: cx.ctxs().toplevel.get() as usize },
            value,
            condition: iso.nil,
            call,
            stop: 0,
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

    /// The value the exit `e` carries (`R_ReturnedValue`, or an error's
    /// message), its condition and its call (R_NilValue where it has none).
    pub fn exit_objects(&self, e: &RError) -> (Ref<'_>, Ref<'_>, Ref<'_>) {
        let _ = e;
        let r = self.top_exit();
        (self.raw_ref(r.value), self.raw_ref(r.condition), self.raw_ref(r.call))
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

/// `maybeClosureWrapper(expr)`: a call of `.Internal`, `.External`,
/// `.External2`, `.External.graphics`, `.Call`, `.Fortran`, `.C` or
/// `.Call.graphics` with a first argument.
// R: eval.c:maybeClosureWrapper
fn maybe_closure_wrapper(cx: &Cx, e: Ref<'_>) -> bool {
    let Some(c) = e.as_cons() else { return false };
    let Some(s) = c.car().as_sym() else { return false };
    let wrapper = matches!(
        s.name().bytes(),
        b".Internal"
            | b".External"
            | b".External2"
            | b".External.graphics"
            | b".Call"
            | b".Fortran"
            | b".C"
            | b".Call.graphics"
    );
    wrapper && c.cdr().as_cons().is_some_and(|a| a.car() != cx.nil())
}

/// `maybePrimitiveCall(expr)`: a call whose function symbol's value in base
/// (`SYMVALUE`, an evaluated promise's value) is a builtin or a special.
// R: eval.c:maybePrimitiveCall
fn maybe_primitive_call(cx: &Cx, e: Ref<'_>) -> bool {
    use robigo_heap::types::*;
    let Some(c) = e.as_cons() else { return false };
    let head = c.car();
    if head.sexptype() != SYMSXP {
        return false;
    }
    // SAFETY: an immortal symbol and base, immortal; the value found is
    // only read for its type.
    unsafe {
        match cx.var_lookup_raw(head.sexp(), cx.base_env().sexp()) {
            crate::RawBinding::Value(v) => {
                let v = if v.typ() == PROMSXP && v.promise_tag() == 0 { v.car() } else { v };
                matches!(v.typ(), BUILTINSXP | SPECIALSXP)
            }
            _ => false,
        }
    }
}
