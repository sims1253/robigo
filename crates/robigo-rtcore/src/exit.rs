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
    /// `Exec` or `Tailcall`'s jump to the function record of the closure
    /// that called it (eval.c's `do_tailcall`), carrying the continuation:
    /// a list of the call, its environment and its function. Where it lands
    /// the record ends as for a `return`, and the exit is pending again,
    /// with no target, for the closure call that made the record to run
    /// (robigo-rt's `tailcall`, GNU R's `handle_exec_continuation`).
    Exec,
}

pub(crate) fn kind_from_u8(k: u8) -> ExitKind {
    match k {
        0 => ExitKind::Error,
        1 => ExitKind::Jump,
        2 => ExitKind::Restart,
        4 => ExitKind::Exec,
        // (an unknown byte decodes as the inert Interrupt, never as Exec)
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
        self.error_call_bytes(call, msg.as_bytes())
    }

    /// [`Cx::error_call`] of a message in the native encoding's bytes, as
    /// GNU R's `char *` messages are (text an R string gives, which need
    /// not be valid UTF-8).
    pub fn error_call_bytes(&mut self, call: Option<Root<'_>>, msg: &[u8]) -> RError {
        let call = call.map_or(self.iso().nil, |c| c.bind(self).sexp());
        self.error_call_sexp(call, msg)
    }

    /// Signal an error reported against the call of the current context,
    /// GNU R's `error()`: the innermost record as R sees the chain, or the
    /// one below it if that is a `CTXT_BUILTIN` record (a foreign call's).
    pub fn error(&mut self, msg: &str) -> RError {
        self.error_bytes(msg.as_bytes())
    }

    /// [`Cx::error`] of a message in the native encoding's bytes
    /// ([`Cx::error_call_bytes`]). GNU R formats `error()`'s message into
    /// `min(BUFSIZE, R_WarnLength)` bytes, its NUL included, so a longer one
    /// is cut there (999 bytes at `warning.length`'s default), short of a
    /// character it would split.
    // R: errors.c:Rf_error, errors.c:Rvsnprintf_mbcs
    pub fn error_bytes(&mut self, msg: &[u8]) -> RError {
        let size = ERROR_BUFSIZE.min(self.report_options().warn_length.get());
        let msg = vsnprintf_cut(msg, size, self.locale().utf8);
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
        if !self.iso().contexts().bc_active.get() {
            return g.call();
        }
        self.bc_interpreter_expression()
    }

    /// The call `error()` and `warning()` report under a `CTXT_CCODE`
    /// context begun with the call R_NilValue (datetime.c's
    /// `prepare_reset_tz`): `getCurrentCall` finds that context first, so
    /// while byte code runs the expression it evaluates
    /// (`R_getBCInterpreterExpression`), else NULL.
    // R: errors.c:getCurrentCall
    pub fn ccode_context_call(&self) -> Ref<'_> {
        if self.iso().contexts().bc_active.get() { self.bc_interpreter_expression() } else { self.nil() }
    }

    /// [`Cx::error`] under such a context ([`Cx::ccode_context_call`]).
    pub fn error_in_ccode_context(&mut self, msg: &str) -> RError {
        let call = self.ccode_context_call().sexp();
        self.error_call_sexp(call, msg.as_bytes())
    }

    /// `R_getBCInterpreterExpression()`: the expression the running byte
    /// code evaluates, as the AST interpreter would report it.
    // R: eval.c:R_getBCInterpreterExpression
    fn bc_interpreter_expression(&self) -> Ref<'_> {
        let g = self.global_context();
        let ctx = self.iso().contexts();
        // (a traced root of the context state: a constant of the running
        // code object, live)
        let e = self.raw_ref(ctx.bc_expr.get());
        if ctx.bc_expr_final.get() {
            return e;
        }
        // an assignment call is shown: `x <<- v` as itself, where
        // inflateAssignmentCall leaves it as it is (its last argument is
        // not tagged `value`). INTERIM(C1): one it would inflate (a direct
        // `` `f<-`(x, value = v) ``, GNU R's `f(x) <- v`) is not made here,
        // under `&Cx` on an error's path; it falls through to the
        // function's call, as before this check (robigo-rt's complex
        // assignments set theirs inflated, `set_bc_expression_final`)
        if maybe_assignment_call(e) && !inflates(e) {
            return e;
        }
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
        let nil = self.iso().nil;
        let (active, expr, fin) = {
            let c = self.iso().contexts();
            let saved = (c.bc_active.get(), c.bc_expr.get(), c.bc_expr_final.get());
            c.bc_active.set(bytecode);
            c.bc_expr.set(nil);
            c.bc_expr_final.set(false);
            saved
        };
        if expr == nil {
            let r = f(self);
            let c = self.iso().contexts();
            c.bc_active.set(active);
            c.bc_expr.set(nil);
            c.bc_expr_final.set(fin);
            return r;
        }
        // an activation begun while the outer one is at an expression (a
        // callback of the builtin it calls): the expression is rooted
        // meanwhile, so a moving collection updates it
        self.scope(|cx, s| {
            let e = s.root(cx, cx.raw_ref(expr));
            let r = f(cx);
            let c = cx.iso().contexts();
            c.bc_active.set(active);
            c.bc_expr.set(e.bind(cx).sexp());
            c.bc_expr_final.set(fin);
            r
        })
    }

    /// The expression the byte code of the innermost activation evaluates
    /// (R_NilValue: none known), for [`Cx::error_context_call`].
    ///
    /// # Safety
    /// `e` is R_NilValue or a live object reachable from the running code
    /// object; it is traced while set.
    #[inline(always)]
    pub unsafe fn set_bc_expression(&self, e: Sexp) {
        let c = self.iso().contexts();
        c.bc_expr.set(e);
        c.bc_expr_final.set(false);
    }

    /// [`Cx::set_bc_expression`], with `fin`: `e` is reported as it is
    /// (`R_getBCInterpreterExpression`'s inflated assignment call).
    ///
    /// # Safety
    /// As for [`Cx::set_bc_expression`].
    #[inline(always)]
    pub unsafe fn set_bc_expression_final(&self, e: Sexp, fin: bool) {
        let c = self.iso().contexts();
        c.bc_expr.set(e);
        c.bc_expr_final.set(fin);
    }

    /// [`Cx::error_call`] with the call as an object of a live record or
    /// root, read in this interval.
    /// Signal the classed error `cond` reported against `call` (`None`:
    /// `R_NilValue`): GNU R's `R_makeErrorCondition(call, ...)` signalled by
    /// `R_signalErrorConditionEx`, through the isolate's hook
    /// ([`crate::Hooks::error_condition`]): the handlers for the
    /// condition's classes, then the default handling of its message.
    // R: errors.c:R_makeErrorCondition, R_signalErrorConditionEx
    pub fn error_condition_call(&mut self, call: Option<Root<'_>>, cond: &crate::ErrorCond<'_>) -> RError {
        let hook = self.iso().hooks.get().error_condition;
        hook(self, call, cond)
    }

    /// [`Cx::error_condition_call`] reported against the current call, as
    /// [`Cx::error`] (GNU R's `R_CurrentExpression` as the call: "behave
    /// like error()").
    pub fn error_condition(&mut self, cond: &crate::ErrorCond<'_>) -> RError {
        let call = self.error_context_call().sexp();
        self.error_condition_sexp(call, cond)
    }

    pub(crate) fn error_condition_sexp(&mut self, call: Sexp, cond: &crate::ErrorCond<'_>) -> RError {
        let hook = self.iso().hooks.get().error_condition;
        let nil = self.iso().nil;
        self.scope(|cx, s| {
            let c = (call != nil).then(|| s.root(cx, cx.raw_ref(call)));
            hook(cx, c, cond)
        })
    }

    /// `R_MissingArgError_c(arg, call, subclass)`: the `missingArgError`
    /// of subclass `subclass`, "argument \"`arg`\" is missing, with no
    /// default" ("argument is missing, with no default" for an empty name),
    /// reported against `call` (`None`: the current call, GNU R's
    /// `R_CurrentExpression`).
    // R: errors.c:R_MissingArgError_c
    #[cold]
    pub fn missing_arg_error(&mut self, arg: &str, call: Option<Root<'_>>, subclass: &'static str) -> RError {
        let msg = if arg.is_empty() {
            "argument is missing, with no default".to_owned()
        } else {
            format!("argument \"{arg}\" is missing, with no default")
        };
        let cond = crate::ErrorCond {
            class: "missingArgError",
            subclass: Some(subclass),
            msg: msg.as_bytes(),
            fields: &[],
            exit_only: false,
        };
        match call {
            Some(c) => self.error_condition_call(Some(c), &cond),
            None => self.error_condition(&cond),
        }
    }

    pub(crate) fn error_call_sexp(&mut self, call: Sexp, msg: &[u8]) -> RError {
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
    pub fn record_error_call(&mut self, call: Option<Root<'_>>, msg: &[u8]) -> RError {
        let cx = &*self;
        let iso = cx.iso();
        let call = call.map_or(iso.nil, |c| c.bind(cx).sexp());
        // A message vector that cannot be allocated leaves the value
        // R_NilValue: the error is still recorded.
        let msg = crate::c_str_bytes(msg);
        // (bytes that are not UTF-8 are the native encoding's)
        let enc = if std::str::from_utf8(msg).is_ok() { crate::Encoding::Utf8 } else { crate::Encoding::Native };
        let value = cx
            .mk_char(msg, enc)
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
        if e.sexptype() == crate::fresh::EMBEDDED_NUL {
            return Err(self.embedded_nul_error());
        }
        let iso = self.iso();
        // SAFETY: `&mut Cx` ends every `Ref`, view and slice of this
        // capability, so everything Rust code still needs is in a root of
        // this isolate (the minting rule, lib.rs). Every root edge is null,
        // immortal or a live object of this heap (every write into a root is
        // owner-checked). No other reference into the core or the roots is
        // live, and the collection reaches nothing of ours (finalizers are
        // only queued).
        let r = unsafe { iso.core().make_room(e, &mut iso.gc_roots()) };
        self.flush_gc_report();
        r.map_err(|a| self.error_call(None, &a.message()))
    }

    /// GNU R's `error(_("embedded nul in string: '%s'"), EncodeString(c, 0,
    /// 0, Rprt_adj_none))` for the string [`Cx::mk_char`] refused.
    // R: envir.c:mkCharLenCE
    fn embedded_nul_error(&mut self) -> RError {
        let (b, enc) = self.iso().embedded_nul.take().expect("robigo: an embedded-NUL refusal without its string");
        let shown = (self.iso().hooks.get().encode_string)(self, &b, enc);
        let msg = format!("embedded nul in string: '{}'", String::from_utf8_lossy(&shown));
        self.error(&msg)
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
        s.name_bytes(),
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

/// `maybeAssignmentCall(expr)`: a call whose function is a symbol of more
/// than two bytes ending in `<-` (`<<-`, `names<-`; not `<-` itself).
// R: eval.c:maybeAssignmentCall
fn maybe_assignment_call(e: Ref<'_>) -> bool {
    let Some(c) = e.as_cons() else { return false };
    if e.sexptype() != robigo_heap::types::LANGSXP {
        return false;
    }
    let Some(s) = c.car().as_sym() else { return false };
    let name = s.name_bytes();
    name.len() > 2 && name.ends_with(b"<-")
}

/// Whether `inflateAssignmentCall(expr)` makes a new call of `expr`, an
/// assignment call: it has two arguments or more and its last is tagged
/// `value`.
// R: eval.c:inflateAssignmentCall
fn inflates(e: Ref<'_>) -> bool {
    let mut args = e.as_cons().map(|c| c.cdr());
    let mut n = 0;
    let mut last_value = false;
    while let Some(a) = args.and_then(|a| a.as_cons()) {
        n += 1;
        last_value = a.tag().as_sym().is_some_and(|t| t.name_bytes() == b"value");
        args = Some(a.cdr());
    }
    n >= 2 && last_value
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

/// GNU R's `BUFSIZE` (errors.c): the size of `error()`'s buffer.
pub(crate) const ERROR_BUFSIZE: usize = 8192;

/// What `Rvsnprintf_mbcs(buf, size, "%s", msg)` leaves in `buf`: at most
/// `size - 1` bytes, then (`mbcsTruncateToValid`, in a UTF-8 locale; the C
/// locale is not multibyte) without a last character the cut left
/// incomplete or invalid.
// R: errors.c:Rvsnprintf_mbcs, util.c:mbcsTruncateToValid
pub(crate) fn vsnprintf_cut(msg: &[u8], size: usize, utf8: bool) -> &[u8] {
    if msg.len() < size {
        return msg;
    }
    let b = &msg[..size.saturating_sub(1)];
    if !utf8 || b.is_empty() {
        return b;
    }
    // back to the last byte that is not a continuation byte
    let mut good = b.len() - 1;
    while good > 0 && (b[good] & 0xC0) == 0x80 {
        good -= 1;
    }
    // mbrtowc of the rest, character by character: up to the first that is
    // incomplete or invalid
    let used = match std::str::from_utf8(&b[good..]) {
        Ok(_) => b.len() - good,
        Err(e) => e.valid_up_to(),
    };
    &b[..good + used]
}

#[cfg(test)]
mod cut_tests {
    use super::vsnprintf_cut;

    #[test]
    fn error_buffer_cut() {
        assert_eq!(vsnprintf_cut(b"abc", 4, true), b"abc");
        assert_eq!(vsnprintf_cut(b"abcd", 4, true), b"abc");
        // "a\u00e9" cut inside the 2-byte character drops it
        assert_eq!(vsnprintf_cut("ab\u{e9}".as_bytes(), 4, true), b"ab");
        assert_eq!(vsnprintf_cut("ab\u{e9}x".as_bytes(), 5, true), "ab\u{e9}".as_bytes());
        // from the last lead byte only, as mbcsTruncateToValid scans: an
        // invalid byte before it stays, one after a whole character goes
        assert_eq!(vsnprintf_cut(b"a\x80bcd", 5, true), b"a\x80bc");
        assert_eq!(vsnprintf_cut(b"ab\x80d", 4, true), b"ab");
        // the C locale cuts bytes
        assert_eq!(vsnprintf_cut("ab\u{e9}".as_bytes(), 4, false), b"ab\xc3");
    }
}
