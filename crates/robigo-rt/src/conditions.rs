//! The condition system's internals (GNU R's `errors.c`): the handler and
//! restart stacks' entries, `stop`, `warning` and the `.Internal`s
//! `conditions.R` runs on (`.addCondHands`, `.signalCondition`,
//! `.invokeRestart`, ...), and the default handlers: the error report
//! (`verrorcall_dflt`, with its "Calls:" traceback), the warning's
//! (`vwarningcall_dflt` under `options(warn)`), the deferred warnings'
//! printing (`PrintWarnings`) and the jump to the top level.
//!
//! # Where conditions are signalled
//!
//! Everywhere GNU R signals them, at once (docs/g3.md §6.5): an error
//! raised anywhere below `rt` (`Cx::error_call`, `Cx::error`) reaches
//! [`error_hook`] through the isolate's hook table before any exit
//! leaves the code that raised it, so calling handlers run with the stack
//! intact, an exiting handler is a jump to its frame, and the default
//! report is written before any `on.exit` code runs. A warning reaches
//! [`warning_hook`] the same way. The token the error hook returns stands
//! for the exit that leaves: the jump to the handler, a restart a handler
//! invoked, or the jump to the top level after the report.
//!
//! # The stacks
//!
//! `R_HandlerStack` and `R_RestartStack` are the isolate's (rtcore's
//! contexts save them in every record and restore them at every exit, as
//! GNU R's `begincontext`/`R_restore_globals` do). A handler entry is GNU
//! R's: a VECSXP of five (class, calling environment, handler, target
//! environment, result) whose `LEVELS` is 1 for a calling handler.
//!
//! # State
//!
//! GNU R's statics (`errbuf`, `inError`, `inWarning`, `immediateWarning`,
//! the two tokens) are per isolate, in [`CondState`].

#![allow(unsafe_code)]

use crate::prims;
use robigo_heap::types::*;
use robigo_rtcore::RSym;
use robigo_rtcore::{Args, Char, Cx, Encoding, Global, Out, RError, RResult, Ref, Root, Scope, ctxt};
use std::cell::{Cell, RefCell};

/// `BUFSIZE`: errors.c's message buffers.
pub const BUFSIZE: usize = 8192;
/// `LONGWARN`: the width at which a message moves to the next line.
const LONGWARN: usize = 75;
/// `R_MB_CUR_MAX` in the oracle's UTF-8 locale.
const MB_CUR_MAX: usize = 6;
/// errors.c's `RESULT_SIZE`: an exiting handler's result vector.
const RESULT_SIZE: usize = 4;

/// The condition system's per-isolate state: GNU R's statics and C
/// globals of `errors.c` and the options that mirror them.
pub struct CondState {
    /// The symbols S3 dispatch names.
    pub s3: crate::dispatch::S3Syms,
    /// `R_HandlerResultToken`: the last element of an exiting handler's
    /// result vector.
    pub handler_result_token: Global,
    /// `errbuf`: the last error's report (or, while handlers run, its
    /// message); `geterrmessage()`.
    pub errbuf: RefCell<Vec<u8>>,
    /// `inError`: 1 while an error is reported, 2 while the traceback is
    /// saved, 3 while `options(error)` runs.
    pub in_error: Cell<i32>,
    /// `inWarning`, `inPrintWarnings`, `immediateWarning`,
    /// `noBreakWarning`.
    pub in_warning: Cell<bool>,
    pub in_print_warnings: Cell<bool>,
    pub immediate_warning: Cell<bool>,
    pub no_break_warning: Cell<bool>,
    // (`R_ShowErrorMessages`, `R_ShowErrorCalls`, `R_ShowWarnCalls`,
    // `R_NShowCalls` and `R_WarnLength` are the isolate's
    // `Cx::report_options`, which `options()` sets)
    /// R_LookupMethod's memo and installS3Signature's (dispatch.rs).
    pub method_memo: RefCell<crate::dispatch::MethodMemo>,
    pub sig_memo: RefCell<crate::dispatch::SigMemo>,
    /// usemethod's methods by (generic, class vector, defining environment).
    pub dispatch_memo: RefCell<crate::dispatch::DispatchMemo>,
    /// The internal generics' primitives by name (`DispatchOrEval`'s `op`),
    /// as offsets into the primitive table.
    pub prim_names: RefCell<Vec<(String, usize)>>,
    /// Frame shapes with neither `.packageName` nor `.__NAMESPACE__.`
    /// among their symbols (shapes never change): a slot frame of one, with
    /// no overflow block and no attributes, is not `topenv`'s answer.
    pub topenv_shapes: RefCell<Vec<Global>>,
    /// Method frame shapes by their code's shape (dispatch.rs,
    /// `method_shape`).
    pub method_shapes: RefCell<crate::dispatch::MethodShapes>,
    /// Match plans of methods called with their generics' arguments
    /// (args.rs, `method_plan_get`).
    pub(crate) method_plans: RefCell<crate::args::MethodPlans>,
    /// dispatchMethod's `static int option` (R_USEMETHOD_FORWARD_LOCALS):
    /// -1 until the first generic with a local that is not a formal.
    pub forward_locals: Cell<i8>,
    /// errors.c's `trycatch_callback` and `wceh_callback`, the functions
    /// C-level tryCatch calls back through, made on first use
    /// (`R_PreserveObject`).
    c_callbacks: RefCell<[Option<Global>; 2]>,
}

impl CondState {
    /// The state of a new session.
    pub fn new(cx: &mut Cx) -> CondState {
        // R: errors.c:do_addCondHands, `allocVector(VECSXP, 1)`
        let token = cx.alloc_vector_raw(VECSXP, 1).expect("robigo: no room for the condition system");
        // SAFETY: fresh, of this heap.
        let handler_result_token = cx.global(unsafe { Ref::from_sexp(token, cx) });
        CondState {
            s3: crate::dispatch::S3Syms::new(cx),
            handler_result_token,
            errbuf: RefCell::new(Vec::new()),
            in_error: Cell::new(0),
            in_warning: Cell::new(false),
            in_print_warnings: Cell::new(false),
            immediate_warning: Cell::new(false),
            no_break_warning: Cell::new(false),
            method_memo: RefCell::new(Default::default()),
            sig_memo: RefCell::new(Default::default()),
            dispatch_memo: RefCell::new(Default::default()),
            prim_names: RefCell::new(Vec::new()),
            topenv_shapes: RefCell::new(Vec::new()),
            method_shapes: RefCell::new(Default::default()),
            method_plans: RefCell::new(Default::default()),
            forward_locals: Cell::new(-1),
            c_callbacks: RefCell::new([None, None]),
        }
    }
}

/// Install the condition system's hooks (the error, warning, exit-value
/// and dispatch entries of the isolate's hook table). Part of
/// [`crate::install`], after the state is in place.
pub(crate) fn install_hooks(cx: &mut Cx) {
    let mut h = cx.hooks();
    h.error = error_hook;
    h.error_condition = error_condition_hook;
    h.warning_condition = warning_condition_hook;
    h.interrupt = onintr_hook;
    h.warning = warning_hook;
    h.exit_value = fixup_exiting_handler_result;
    h.dispatch = crate::dispatch::dispatch_hook;
    cx.install_hooks(h);
}

/// The isolate's condition state.
pub fn state(cx: &Cx) -> &CondState {
    &prims::state(cx).cond
}

// ---- small helpers --------------------------------------------------------------

/// The string's bytes truncated to at most `n` bytes at a UTF-8 character
/// boundary (`mbcsTruncateToValid` after a `vsnprintf` into `n + 1`).
fn trunc(b: &[u8], n: usize) -> &[u8] {
    if b.len() <= n {
        return b;
    }
    let mut k = n;
    // back up over continuation bytes
    while k > 0 && (b[k] & 0xC0) == 0x80 {
        k -= 1;
    }
    &b[..k]
}

/// `Rstrncat(dest, src, n)`: append at most `n` bytes, not splitting a
/// character.
fn strncat(dest: &mut Vec<u8>, src: &[u8], n: usize) {
    dest.extend_from_slice(trunc(src, n));
}

/// `ERRBUFCAT(txt)`.
fn errbufcat(buf: &mut Vec<u8>, txt: &[u8]) {
    let n = BUFSIZE.saturating_sub(buf.len());
    strncat(buf, txt, n);
}

/// `wd(buf)`: the display width of UTF-8 text.
pub fn wd(b: &[u8]) -> usize {
    String::from_utf8_lossy(b).chars().map(|c| robigo_text::ri18n_wcwidth(c as u32).max(0) as usize).sum()
}

/// The first line of `b`.
fn line1(b: &[u8]) -> &[u8] {
    b.split(|&c| c == b'\n').next().unwrap_or(b)
}

/// `CHAR(STRING_ELT(deparse1s(call), 0))`.
fn dcall(cx: &Cx, call: Ref<'_>) -> Vec<u8> {
    match crate::deparse::deparse1s(cx, call) {
        Ok(b) => b,
        Err(m) => format!("<{m}>").into_bytes(),
    }
}

/// Force the unforced promises `call` holds, as deparsing it for a
/// message does (`deparse2buff`'s PROMSXP case evaluates the promise):
/// NextMethod's `fixcall` puts its `...` promises into the method's call.
// R: deparse.c:deparse2buff (PROMSXP)
fn force_call_promises(cx: &mut Cx, call: Root<'_>) -> RResult<()> {
    fn walk<'g, 's>(cx: &'g Cx, s: Scope<'s>, x: Ref<'g>, acc: &mut Vec<Root<'s>>) {
        let mut l = x;
        while let Some(c) = l.as_cons() {
            let a = c.car();
            match a.sexptype() {
                PROMSXP => {
                    if let robigo_rtcore::View::Promise(p) = a.view()
                        && p.value().sexp() == cx.unbound().sexp()
                    {
                        acc.push(s.root(cx, a));
                    }
                }
                LANGSXP => walk(cx, s, a, acc),
                _ => {}
            }
            l = c.cdr();
        }
    }
    if call.bind(cx).sexptype() != LANGSXP {
        return Ok(());
    }
    cx.scope(|cx, s| {
        let mut proms = Vec::new();
        walk(cx, s, call.bind(cx), &mut proms);
        // (one result slot for every force)
        let r = s.out(cx).root();
        for p in proms {
            let o = r.reset(cx);
            crate::eval::force(cx, p, o)?;
        }
        Ok(())
    })
}

/// A rooted symbol.
fn sym<'s>(cx: &mut Cx, s: Scope<'s>, name: &[u8]) -> RResult<Root<'s>> {
    let o = s.out(cx);
    let r = o.root();
    cx.build(o, |cx| Ok(cx.install(name)?.as_ref()))?;
    Ok(r)
}

/// `mkString(b)`, rooted.
fn mk_string<'s>(cx: &mut Cx, s: Scope<'s>, b: &[u8]) -> RResult<Root<'s>> {
    let o = s.out(cx);
    let r = o.root();
    cx.build(o, |cx| cx.mk_string(b, robigo_rtcore::Encoding::Native))?;
    Ok(r)
}

/// `GetOption1(install(name))` (`.Options` is rtbuiltins' options.rs).
fn get_option1<'s>(cx: &mut Cx, s: Scope<'s>, name: &[u8]) -> RResult<Root<'s>> {
    robigo_rtbuiltins::options::get_option(cx, s, name)
}

/// `asInteger(x)` for an option's value: NA for anything not a number.
fn as_integer_opt(x: Ref<'_>) -> Option<i32> {
    match x.len() {
        Some(n) if n >= 1 => {}
        _ => return None,
    }
    if let Some(v) = x.as_vector::<i32>() {
        let i = v.get(0);
        return (i != i32::MIN).then_some(i);
    }
    if let Some(v) = x.as_vector::<f64>() {
        let d = v.get(0);
        if d.is_nan() || d >= 2147483648.0 || d <= -2147483649.0 {
            return None;
        }
        return Some(d as i32);
    }
    if let Some(v) = x.as_vector::<robigo_rtcore::Lgl>() {
        let l = v.get(0);
        return (l != robigo_rtcore::Lgl::NA).then_some(l.0);
    }
    None
}

/// Evaluate `call` in `env` keeping `R_Visible` (`evalKeepVis`); the
/// value dropped.
fn eval_keep_vis(cx: &mut Cx, call: Root<'_>, env: Root<'_>) -> RResult<()> {
    let vis = cx.visible();
    let r = cx.scope(|cx, s| {
        let o = s.out(cx);
        crate::eval(cx, call, env, o)
    });
    cx.set_visible(vis);
    r
}

/// `lang3(R_DoubleColonSymbol, R_BaseSymbol, R_QuoteSymbol)` applied to
/// `call`: `base::quote(call)`, rooted.
fn base_quote<'s>(cx: &mut Cx, s: Scope<'s>, call: Option<Root<'_>>) -> RResult<Root<'s>> {
    let dc = sym(cx, s, b"::")?;
    let base = sym(cx, s, b"base")?;
    let quote = sym(cx, s, b"quote")?;
    let o = s.out(cx);
    let r = o.root();
    cx.build(o, |cx| {
        let nil = cx.nil();
        let qfun = cx.lcons(dc.bind(cx), cx.cons(base.bind(cx), cx.cons(quote.bind(cx), nil)?)?)?;
        let c = call.map_or(nil, |c| c.bind(cx));
        cx.lcons(qfun, cx.cons(c, nil)?)
    })?;
    Ok(r)
}

/// The handler-stack entry's fields.
mod entry {
    pub const CLASS: usize = 0;
    pub const HANDLER: usize = 2;
    pub const TARGET_ENVIR: usize = 3;
    pub const RETURN_RESULT: usize = 4;
}

/// `ENTRY_*(e)`: field `i` of a handler entry.
fn entry_elt<'g>(e: Ref<'g>, i: usize) -> Ref<'g> {
    e.as_vector::<Ref<'_>>().expect("robigo: a handler entry that is not a list").get(i)
}

/// `IS_CALLING_ENTRY(e)`: its `LEVELS`.
fn is_calling(e: Ref<'_>) -> bool {
    // SAFETY: a live object; gp is a header field.
    unsafe { e.sexp().gp() != 0 }
}

/// The handler stack's first entry whose class `pred` accepts, as the
/// stack cell (`list`) holding it.
fn find_handler<'g>(cx: &'g Cx, pred: impl Fn(&[u8]) -> bool) -> Option<Ref<'g>> {
    let mut l = cx.handler_stack();
    while let Some(c) = l.as_cons() {
        let class = entry_elt(c.car(), entry::CLASS);
        if let Some(ch) = class.as_char()
            && pred(ch.bytes())
        {
            return Some(l);
        }
        l = c.cdr();
    }
    None
}

/// `R_HandlerStack = CDR(list)`.
fn pop_handlers_to(cx: &mut Cx, list: Root<'_>) {
    cx.scope(|cx, s| {
        let rest = list.bind(cx).as_cons().expect("robigo: a handler stack cell").cdr();
        let r = s.root(cx, rest);
        cx.set_handler_stack(r);
    });
}

// ---- errors ---------------------------------------------------------------------

/// The isolate's error hook: GNU R's `errorcall(call, "%s", msg)`, the
/// handler walk (`vsignalError`) then the default handling
/// (`verrorcall_dflt`). Returns the token of the exit that leaves.
// R: errors.c:errorcall
pub fn error_hook(cx: &mut Cx, call: Option<Root<'_>>, msg: &[u8]) -> RError {
    let msg = robigo_rtcore::c_str_bytes(msg);
    if let Some(e) = signal_error(cx, call, msg) {
        return e;
    }
    verrorcall_dflt(cx, call, msg)
}

/// The isolate's classed-error hook: GNU R's `R_makeErrorCondition(call,
/// class, subclass, nextra, msg)` with its fields set, then
/// `R_signalErrorConditionEx(cond, call, exitOnly)`: the handlers for the
/// condition's classes, the handler stack restored after them, then the
/// default handling of its message.
///
/// GNU R makes the expression, node and protect stack overflows' conditions once
/// (`R_InitConditions`, with a NULL call); one is made here per signal,
/// with the same value.
// R: errors.c:R_signalErrorConditionEx, R_vmakeErrorCondition
pub fn error_condition_hook(cx: &mut Cx, call: Option<Root<'_>>, c: &robigo_rtcore::ErrorCond<'_>) -> RError {
    // Rvsnprintf_mbcs(emsg_buf, BUFSIZE, ...)
    let msg = trunc(robigo_rtcore::c_str_bytes(c.msg), BUFSIZE - 1).to_vec();
    let preset = matches!(
        c.subclass,
        Some("expressionStackOverflowError" | "nodeStackOverflowError" | "protectStackOverflowError")
    );
    cx.scope(|cx, s| {
        let call = if preset { None } else { call };
        let cond = match make_condition(cx, s, call, CondBase::Error, c.class, c.subclass, c.fields, &msg) {
            Ok(cond) => cond,
            Err(e) => return e,
        };
        let oldstack = s.root(cx, cx.handler_stack());
        if let Some(e) = signal_condition_c(cx, cond, call, c.exit_only) {
            return e;
        }
        cx.set_handler_stack(oldstack);
        verrorcall_dflt(cx, call, &msg)
    })
}

/// The isolate's classed-warning hook: GNU R's `R_makeWarningCondition(
/// call, class, subclass, nextra, msg)` with its fields set, then
/// `R_signalWarningCondition(cond)`: base's `warning(cond)` evaluated in a
/// new environment enclosed by the base namespace, `R_Visible` kept: the
/// condition goes through `warning()`'s R code, as GNU R's does.
// R: errors.c:R_vmakeWarningCondition, R_signalWarningCondition
pub fn warning_condition_hook(cx: &mut Cx, call: Option<Root<'_>>, c: &robigo_rtcore::WarningCond<'_>) -> RResult<()> {
    // Rvsnprintf_mbcs(emsg_buf, BUFSIZE, ...)
    let msg = trunc(robigo_rtcore::c_str_bytes(c.msg), BUFSIZE - 1).to_vec();
    cx.scope(|cx, s| {
        let cond = make_condition(cx, s, call, CondBase::Warning, c.class, c.subclass, c.fields, &msg)?;
        let cond_sym = sym(cx, s, b"cond")?;
        let warning_sym = sym(cx, s, b"warning")?;
        let o = s.out(cx);
        let expr = o.root();
        cx.build(o, |cx| {
            let a = cx.cons(cond_sym.bind(cx), cx.nil())?;
            cx.lcons(warning_sym.bind(cx), a)
        })?;
        let ns = s.root(cx, cx.base_namespace());
        let o = s.out(cx);
        let env = o.root();
        cx.build(o, |cx| cx.new_env(ns.bind(cx), false, 0))?;
        cx.define_var(cond_sym, cond, env)?;
        eval_keep_vis(cx, expr, env)
    })
}

/// The base class of a condition [`make_condition`] builds: GNU R's two
/// builders, `R_vmakeErrorCondition`'s "error" and
/// `R_vmakeWarningCondition`'s "warning".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CondBase {
    Error,
    Warning,
}

impl CondBase {
    /// The class after the condition's own, before "condition".
    fn class(self) -> &'static str {
        match self {
            CondBase::Error => "error",
            CondBase::Warning => "warning",
        }
    }
}

/// `R_vmakeErrorCondition` / `R_vmakeWarningCondition(call, class,
/// subclass, nextra, msg)` (by `base`) and
/// `R_setConditionField` of each of `fields`: the list `(message, call,
/// fields...)`, named, of class `c(subclass, class, base, "condition")`.
// R: errors.c:R_vmakeErrorCondition, R_vmakeWarningCondition, R_setConditionField
#[allow(clippy::too_many_arguments)]
fn make_condition<'s>(
    cx: &mut Cx,
    s: Scope<'s>,
    call: Option<Root<'_>>,
    base: CondBase,
    class: &str,
    subclass: Option<&str>,
    fields: &[(&'static str, Root<'_>)],
    msg: &[u8],
) -> RResult<Root<'s>> {
    let o = s.out(cx);
    let r = o.root();
    cx.build(o, |cx| {
        let n = 2 + fields.len();
        let m = cx.mk_string(msg, Encoding::Native)?;
        let mut names = cx.alloc_vector::<Char<'_>>(n)?;
        names.set(0, cx.mk_char(b"message", Encoding::Native)?);
        names.set(1, cx.mk_char(b"call", Encoding::Native)?);
        for (i, (nm, _)) in fields.iter().enumerate() {
            names.set(2 + i, cx.mk_char(nm.as_bytes(), Encoding::Native)?);
        }
        let names = names.finish();
        let classes: Vec<&str> = subclass.into_iter().chain([class, base.class(), "condition"]).collect();
        let mut klass = cx.alloc_vector::<Char<'_>>(classes.len())?;
        for (i, k) in classes.iter().enumerate() {
            klass.set(i, cx.mk_char(k.as_bytes(), Encoding::Native)?);
        }
        let klass = klass.finish();
        let mut l = cx.alloc_vector::<Ref<'_>>(n)?;
        l.set(0, m);
        l.set(1, call.map_or(cx.nil(), |c| c.bind(cx)));
        for (i, (_, v)) in fields.iter().enumerate() {
            l.set(2 + i, v.bind(cx));
        }
        // setAttrib(cond, R_NamesSymbol, names), then the class
        let attrs = cx.cons_tagged(klass.as_ref(), cx.nil(), cx.sym(RSym::Class))?;
        let attrs = cx.cons_tagged(names.as_ref(), attrs, cx.sym(RSym::Names))?;
        l.set_attrib(attrs);
        l.set_object(true);
        Ok(l.finish().into())
    })?;
    Ok(r)
}

/// `R_signalCondition(cond, call, FALSE, exitOnly)`: each handler for the
/// condition's classes, innermost first, each popped as it is tried. A
/// calling handler is called with the condition in the global environment
/// (skipped under `exit_only`); the restart token ends the walk (to the
/// default handling); an exiting handler is a jump. `Some`: the exit that
/// leaves; `None`: the walk ended (the caller restores the stack).
// R: errors.c:R_signalCondition
fn signal_condition_c(cx: &mut Cx, cond: Root<'_>, call: Option<Root<'_>>, exit_only: bool) -> Option<RError> {
    cx.scope(|cx, s| {
        while let Some(list) = find_condition_handler(cx, cond.bind(cx)) {
            let list = s.root(cx, list);
            let entry = s.root(cx, list.bind(cx).as_cons().unwrap().car());
            pop_handlers_to(cx, list);
            if is_calling(entry.bind(cx)) {
                let h = entry_elt(entry.bind(cx), entry::HANDLER);
                if h.sexp() == cx.restart_token().sexp() {
                    return None;
                }
                if !exit_only {
                    let h = s.root(cx, h);
                    if let Err(e) = cx.check_c_stack() {
                        return Some(e);
                    }
                    let r = (|| -> RResult<()> {
                        let o = s.out(cx);
                        let hcall = o.root();
                        cx.build(o, |cx| {
                            let a = cx.cons(cond.bind(cx), cx.nil())?;
                            cx.lcons(h.bind(cx), a)
                        })?;
                        let g = s.root(cx, cx.global_env());
                        let o = s.out(cx);
                        crate::eval(cx, hcall, g, o)
                    })();
                    if let Err(e) = r {
                        return Some(e);
                    }
                }
            } else {
                return Some(goto_exiting_handler(cx, cond, call, entry));
            }
        }
        None
    })
}

/// `vsignalError(call, msg)`: the handlers for simple errors, innermost
/// first, each popped as it is tried (the stack is not restored: an exit
/// restores it). A calling handler runs `.handleSimpleError(h, msg,
/// base::quote(call))` in base; an exiting one is a jump to its frame.
/// `Some`: the exit that leaves (a handler's, or the jump); `None`: no
/// handler took the error, or a restart-token entry sent it to the
/// default handling.
// R: errors.c:vsignalError
fn signal_error(cx: &mut Cx, call: Option<Root<'_>>, msg: &[u8]) -> Option<RError> {
    let localbuf = trunc(msg, BUFSIZE - 2).to_vec();
    loop {
        let r = cx.scope(|cx, s| {
            let list = find_handler(cx, |c| c == b"simpleError" || c == b"error" || c == b"condition")?;
            let list = s.root(cx, list);
            let entry = s.root(cx, list.bind(cx).as_cons().unwrap().car());
            pop_handlers_to(cx, list);
            *state(cx).errbuf.borrow_mut() = trunc(&localbuf, BUFSIZE - 1).to_vec();
            if is_calling(entry.bind(cx)) {
                let h = entry_elt(entry.bind(cx), entry::HANDLER);
                if h.sexp() == cx.restart_token().sexp() {
                    // go to default error handling
                    return Some(None);
                }
                // while a C stack overflow is handled (R_OldCStackLimit),
                // every calling handler counts as failed
                if cx.c_stack_overflow_pending() {
                    return Some(Some(None));
                }
                let h = s.root(cx, h);
                let r = (|| -> RResult<()> {
                    let hooksym = sym(cx, s, b".handleSimpleError")?;
                    let qcall = base_quote(cx, s, call)?;
                    let m = mk_string(cx, s, &localbuf)?;
                    let o = s.out(cx);
                    let hcall = o.root();
                    cx.build(o, |cx| {
                        let nil = cx.nil();
                        let a = cx.cons(qcall.bind(cx), nil)?;
                        let a = cx.cons(m.bind(cx), a)?;
                        let a = cx.cons(h.bind(cx), a)?;
                        cx.lcons(hooksym.bind(cx), a)
                    })?;
                    let base = s.root(cx, cx.base_env());
                    let o = s.out(cx);
                    crate::eval(cx, hcall, base, o)
                })();
                match r {
                    Ok(()) => Some(Some(None)),
                    Err(e) => Some(Some(Some(e))),
                }
            } else {
                let nil = s.root(cx, cx.nil());
                Some(Some(Some(goto_exiting_handler(cx, nil, call, entry))))
            }
        });
        match r {
            // no handler left
            None => return None,
            // the restart token: default handling
            Some(None) => return None,
            // a calling handler returned: the next one
            Some(Some(None)) => continue,
            Some(Some(Some(e))) => return Some(e),
        }
    }
}

/// `gotoExitingHandler(cond, call, entry)`: the handler's result vector
/// filled (condition, call, handler), then `findcontext(CTXT_FUNCTION,
/// rho, result)` to the frame that established it.
// R: errors.c:gotoExitingHandler
fn goto_exiting_handler(cx: &mut Cx, cond: Root<'_>, call: Option<Root<'_>>, entry: Root<'_>) -> RError {
    cx.scope(|cx, s| {
        let e = entry.bind(cx);
        let rho = s.root(cx, entry_elt(e, entry::TARGET_ENVIR));
        let result = s.root(cx, entry_elt(e, entry::RETURN_RESULT));
        let handler = entry_elt(e, entry::HANDLER).sexp();
        let call = call.map_or(cx.nil().sexp(), |c| c.bind(cx).sexp());
        // SAFETY: the entry's result is a VECSXP of RESULT_SIZE that
        // do_addCondHands made (rooted); the stored objects are live.
        unsafe {
            let r = result.bind(cx).sexp();
            if r.typ() == VECSXP && r.std_len() as usize >= 3 {
                r.set_elt(0, cond.bind(cx).sexp());
                r.set_elt(1, call);
                r.set_elt(2, handler);
            }
        }
        // (try_cmpfun's entry: its record is native, not a function's)
        let mask = if handler == cx.unbound().sexp() { ctxt::CCODE } else { ctxt::FUNCTION };
        cx.find_context(mask, rho, Some(result))
    })
}

/// cmp.R's `tryCmpfun(f)` and `tryCompile(e)` around a JIT compile, `body`:
/// `tryCatch(cmpfun(f), error = function(e) { notifyCompilerError(paste(
/// e$message, "at", deparse(e$call))); f })`, made here because robigo's
/// compiler is Rust. An exiting handler for class `"error"` is the
/// innermost while `body` runs, so an error inside the compile (an R error
/// of code it runs: a promise it forces, a warning made an error, a
/// handler's `stop()`, the evaluation depth) ends the compile and nothing
/// outside sees it. Then the compiler's message is given (silent under
/// `compilerOptions$suppressAll`, the default) and the result is None: the
/// closure or loop runs uncompiled. Other exits pass: interrupts,
/// restarts, another condition's exiting handler, `return`s.
///
/// The entry is `mkHandlerEntry("error", R_GlobalEnv, <unbound>, target,
/// result, FALSE)`: `goto_exiting_handler` sends an entry whose handler is
/// R_UnboundValue to a `CTXT_CCODE` record whose `cloenv` is `target`, the
/// one this function makes (GNU R's goes to `doTryCatch`'s frame). The
/// record has the innermost record's call, so `error()` and `warning()`
/// report what they report without it.
///
/// The JIT is off while the compile and the handler run, and the level is
/// restored only when they return: an exit that leaves them (a handler's
/// error, a jump to another condition's exiting handler, an interrupt)
/// leaves the level as it is then, 0 unless code they ran changed it, as
/// GNU R's callers set `R_jit_enabled = 0` around `R_cmpfun` and restore
/// it after the call, which a longjmp skips.
///
/// R code that runs inside the compile (a promise it forces, a warning's
/// handler, the compiler's message) sees no R frame of the compile:
/// `sys.call()`, `sys.function()`, `parent.frame()` and `sys.calls()`
/// there see the frames outside the compiled call's (the record is
/// CTXT_CCODE, which they skip, and its fresh `cloenv` is no frame of
/// theirs), where GNU R's see `tryCmpfun`, `tryCatch`, `tryCatchList`,
/// `tryCatchOne`, `doTryCatch` and cmp.R's own frames below the called
/// function's; a traceback of an error that leaves the compile differs
/// the same way (`tests/rt/jit/compile-error-frames.R`, not ratcheted).
/// The record's call is the innermost record's, so `error()` and
/// `warning()` report the call they report without it; cmp.R's warnings
/// carry cmp.R's own calls ([`crate::cenv::raise_compile_warnings`]), as
/// in GNU R.
///
/// INTERIM(V4 step 3): when cmp.R's own code compiles, its `tryCmpfun`
/// runs and this function goes, with its one call per compile entry
/// (compile.rs) and the CCODE case above.
// R: cmp.R:tryCmpfun, tryCompile; conditions.R:tryCatch, tryCatchOne
pub(crate) fn try_cmpfun(
    cx: &mut Cx,
    body: impl FnOnce(&mut Cx) -> RResult<Option<robigo_heap::Sexp>>,
) -> RResult<Option<robigo_heap::Sexp>> {
    cx.scope(|cx, s| {
        let o = s.out(cx);
        let target = o.root();
        cx.build(o, |cx| cx.new_env(cx.empty_env(), false, 0))?;
        let o = s.out(cx);
        let result = o.root();
        cx.build(o, |cx| {
            let mut r = cx.alloc_vector::<Ref<'_>>(RESULT_SIZE)?;
            r.set(RESULT_SIZE - 1, state(cx).handler_result_token.bind(cx));
            Ok(r.finish().into())
        })?;
        let call = s.root(cx, cx.global_context().call());
        let base = s.root(cx, cx.base_env());
        // the JIT off for the compile and its handler (R_execClosure's and
        // R_compileAndExecute's `R_jit_enabled = 0`), restored only when
        // the compile returns: an exit that leaves it leaves the JIT off,
        // as GNU R's longjmp skips the restore
        let jit = &crate::prims::state(cx).jit;
        let saved = jit.get();
        jit.set(crate::jit::Config { level: 0, ..saved });
        let r = cx.with_context(ctxt::CCODE, call, target, base, None, |cx, _| {
            let o = s.out(cx);
            let stack = o.root();
            cx.build(o, |cx| {
                let mut e = cx.alloc_vector::<Ref<'_>>(5)?;
                e.set(0, cx.mk_char(b"error", Encoding::Native)?.as_ref());
                e.set(1, cx.global_env());
                e.set(2, cx.unbound());
                e.set(3, target.bind(cx));
                e.set(4, result.bind(cx));
                // (gp 0: an exiting handler)
                cx.cons(e.finish().as_ref(), cx.handler_stack())
            })?;
            cx.set_handler_stack(stack);
            body(cx)
        });
        let code = match r {
            Ok(code) => code,
            Err(robigo_rtcore::Left::Exit(e)) => return Err(e),
            Err(robigo_rtcore::Left::Landed(_)) => {
                notify_compiler_error(cx, s, result)?;
                None
            }
        };
        crate::prims::state(cx).jit.set(saved);
        Ok(code)
    })
}

/// tryCmpfun's handler after the jump: `tryCatchOne`'s condition from the
/// result vector, then the handler, `function(e) notifyCompilerError(
/// paste(e$message, "at", deparse(e$call)))`, called with it in the
/// compiler's namespace: always, as GNU R's does, so that the option is
/// read as cmp.R reads it then (`compilerOptions$suppressAll`, which `!`
/// and `if` take, their errors included) and a `notifyCompilerError` that
/// was replaced or traced runs. It runs only after a failed compile.
/// Nothing runs when the compiler is not loaded: INTERIM(L5), the source
/// boot's JIT compiles without it, where GNU R's loads it at boot.
// R: cmp.R:tryCmpfun, notifyCompilerError; conditions.R:tryCatchOne
fn notify_compiler_error(cx: &mut Cx, s: Scope<'_>, result: Root<'_>) -> RResult<()> {
    let pkg = sym(cx, s, b"compiler")?;
    let ns = {
        let (Some(reg), Some(y)) = (crate::dispatch::env_ref(cx.namespace_registry()), pkg.bind(cx).as_sym()) else {
            return Ok(());
        };
        match cx.frame_binding(reg, y) {
            robigo_rtcore::Lookup::Bound(b) => match b.get(cx) {
                robigo_rtcore::BindingValue::Value(v) => v.sexp(),
                _ => return Ok(()),
            },
            _ => return Ok(()),
        }
    };
    // SAFETY: a namespace the registry holds.
    let ns = s.root(cx, unsafe { Ref::from_sexp(ns, cx) });
    const NOTIFY: &str = "{
        cond <- value[[1L]]
        if (is.null(cond)) cond <- simpleError(.Internal(geterrmessage()), value[[2L]])
        else if (is.character(cond)) cond <- simpleError(cond, value[[2L]])
        (function(e) notifyCompilerError(paste(e$message, \"at\", deparse(e$call))))(cond)
    }";
    let exprs = s.capture(cx, |cx, o| crate::lower::parse(cx, NOTIFY, o))?;
    let e = s.root(cx, exprs.bind(cx).as_vector::<Ref<'_>>().unwrap().get(0));
    let o = s.out(cx);
    let env = o.root();
    cx.build(o, |cx| cx.new_env(ns.bind(cx), false, 1))?;
    let value = sym(cx, s, b"value")?;
    cx.define_var(value, result, env)?;
    let o = s.out(cx);
    crate::eval(cx, e, env, o)
}

/// `R_FixupExitingHandlerResult(result)`: before `on.exit` code runs while
/// a simple error jumps to its exiting handler, the message (in `errbuf`)
/// is saved into the result, in case the code raises an error that
/// overwrites the buffer.
// R: errors.c:R_FixupExitingHandlerResult
pub fn fixup_exiting_handler_result(cx: &mut Cx, result: Root<'_>) {
    let ok = {
        let r = result.bind(cx);
        let token = state(cx).handler_result_token.bind(cx).sexp();
        r.sexptype() == VECSXP
            && r.as_vector::<Ref<'_>>().is_some_and(|v| {
                v.len() == RESULT_SIZE && v.get(0).sexptype() == NILSXP && v.get(RESULT_SIZE - 1).sexp() == token
            })
    };
    if !ok {
        return;
    }
    cx.scope(|cx, s| {
        let msg = state(cx).errbuf.borrow().clone();
        let Ok(m) = mk_string(cx, s, &msg) else { return };
        // SAFETY: the rooted result vector checked above; `m` is rooted.
        unsafe { result.bind(cx).sexp().set_elt(0, m.bind(cx).sexp()) };
    });
}

/// `R_ConciseTraceback(call, skip)`: the function names of the function
/// and builtin records from the innermost outwards, `a -> b -> c` from the
/// outermost, starting after the last `stop`, `warning`,
/// `suppressWarnings` or `.signalSimpleWarning`; empty when it adds
/// nothing to the call.
// R: errors.c:R_ConciseTraceback
pub fn concise_traceback(cx: &Cx, call: Option<Ref<'_>>, mut skip: usize) -> Vec<u8> {
    let nshow = cx.report_options().n_show_calls.get();
    let mut buf: Vec<u8> = Vec::new();
    let mut ncalls = 0;
    let mut too_many = false;
    let mut top: Vec<u8> = Vec::new();
    let mut c = Some(cx.global_context());
    while let Some(r) = c {
        if r.callflag() == ctxt::TOPLEVEL {
            break;
        }
        if r.callflag() & (ctxt::FUNCTION | ctxt::BUILTIN) != 0 {
            if skip > 0 {
                skip -= 1;
            } else {
                let fun = r.call().as_cons().map(|c| c.car());
                let this: Vec<u8> = match fun.and_then(|f| f.as_sym()) {
                    Some(s) => s.name_bytes().to_vec(),
                    None => b"<Anonymous>".to_vec(),
                };
                if matches!(&this[..], b"stop" | b"warning" | b"suppressWarnings" | b".signalSimpleWarning") {
                    buf.clear();
                    ncalls = 0;
                    too_many = false;
                } else {
                    ncalls += 1;
                    if too_many {
                        top = this;
                    } else if buf.len() > nshow {
                        let mut b = b"... ".to_vec();
                        b.extend_from_slice(&buf);
                        buf = b;
                        too_many = true;
                        top = this;
                    } else if !buf.is_empty() {
                        let mut b = this.clone();
                        b.extend_from_slice(b" -> ");
                        b.extend_from_slice(&buf);
                        buf = b;
                    } else {
                        buf = this;
                    }
                }
            }
        }
        c = r.next();
    }
    if too_many && top.len() < 50 {
        let mut b = top;
        b.push(b' ');
        b.extend_from_slice(&buf);
        buf = b;
    }
    // don't add Calls if it adds no extra information
    if ncalls == 1
        && let Some(call) = call
        && let Some(cell) = call.as_cons()
        && call.sexptype() == LANGSXP
    {
        let this: Vec<u8> = match cell.car().as_sym() {
            Some(s) => s.name_bytes().to_vec(),
            None => b"<Anonymous>".to_vec(),
        };
        if buf == this {
            return Vec::new();
        }
    }
    // (GNU R's buffer holds 560 bytes)
    buf
}

/// `verrorcall_dflt(call, "%s", msg)`: the report written to `errbuf` and
/// shown (`Error in CALL : MSG`, the traceback line, the deferred
/// warnings), then `jump_to_top_ex`.
// R: errors.c:verrorcall_dflt
pub fn verrorcall_dflt(cx: &mut Cx, call: Option<Root<'_>>, msg: &[u8]) -> RError {
    let st = state(cx);
    if st.in_error.get() != 0 {
        // fail-safe handler for recursive errors
        if st.in_error.get() == 3 {
            cx.reprintf(b"Error during wrapup: ");
            let mut b = trunc(msg, BUFSIZE).to_vec();
            b.push(b'\n');
            cx.reprintf(&b);
        }
        if !cx.collected_warnings().is_empty() {
            cx.clear_warnings();
            cx.reprintf(b"Lost warning messages\n");
        }
        cx.reprintf(b"Error: no more error handlers available (recursive errors?); invoking 'abort' restart\n");
        return jump_to_top_ex(cx, false, false, false, false, false, Some((call, msg)));
    }
    let old_in_error = st.in_error.get();
    st.in_error.set(1);
    let warn_length = cx.report_options().warn_length.get();
    // For use with Rv?snprintf, which truncates at size - 1, hence the + 1
    let msg_len = BUFSIZE.min(warn_length) + 1;
    let mut errbuf: Vec<u8> = Vec::new();
    if let Some(c) = call
        && let Err(e) = force_call_promises(cx, c)
    {
        return e;
    }
    let callr = call.map(|c| c.bind(cx)).filter(|c| c.sexptype() != NILSXP);
    if let Some(c) = callr {
        let head = b"Error in ";
        let tail = b"\n  ";
        // (show.error.locations: robigo-rt keeps no srcrefs, so there is
        // never a location to show)
        let dcall = dcall(cx, c);
        let tmp = trunc(msg, msg_len.saturating_sub(head.len()).saturating_sub(1));
        if head.len() + tail.len() + tmp.len() < BUFSIZE {
            errbuf.extend_from_slice(trunc(&[&b"Error in "[..], &dcall, b" : "].concat(), BUFSIZE - 1));
            if 14 + wd(&dcall) + wd(line1(tmp)) > LONGWARN {
                errbufcat(&mut errbuf, tail);
            }
            errbufcat(&mut errbuf, tmp);
        } else {
            errbuf.extend_from_slice(b"Error: ");
            errbufcat(&mut errbuf, tmp);
        }
    } else {
        errbuf.extend_from_slice(b"Error: ");
        let n = msg_len.saturating_sub(errbuf.len()).saturating_sub(1);
        errbuf.extend_from_slice(trunc(msg, n));
    }
    // Approximate truncation detection
    let nc = errbuf.len();
    if nc > BUFSIZE - 1 - (MB_CUR_MAX - 1) {
        let end = (nc + 1).min((BUFSIZE + 1) - 4);
        errbuf.truncate(end - 1);
        let t = trunc(&errbuf, errbuf.len()).len();
        errbuf.truncate(t);
        errbufcat(&mut errbuf, b"...\n");
    } else {
        let mut nc = nc;
        if errbuf.last() != Some(&b'\n') {
            errbufcat(&mut errbuf, b"\n");
            nc += 1;
        }
        if cx.report_options().show_error_calls.get()
            && let Some(c) = callr
        {
            let tr = concise_traceback(cx, Some(c), 0);
            if !tr.is_empty() {
                let call_trans = b"Calls:";
                if tr.len() + nc + call_trans.len() + 2 < BUFSIZE + 1 {
                    errbufcat(&mut errbuf, call_trans);
                    errbufcat(&mut errbuf, b" ");
                    errbufcat(&mut errbuf, &tr);
                    errbufcat(&mut errbuf, b"\n");
                }
            }
        }
    }
    *state(cx).errbuf.borrow_mut() = errbuf.clone();
    let show = cx.report_options().show_error_messages.get();
    if show {
        cx.reprintf(&errbuf);
    }
    if show && !cx.collected_warnings().is_empty() {
        cx.reprintf(b"In addition: ");
        print_warnings(cx);
    }
    let e = jump_to_top_ex(cx, true, true, true, true, false, Some((call, msg)));
    // (the context that restores inError on the way out)
    state(cx).in_error.set(old_in_error);
    e
}

/// `jump_to_top_ex(traceback, tryUserHandler, processWarnings,
/// resetConsole, ignoreRestartContexts)`: `options(error)`, the deferred
/// warnings, a jump to a `browser`/`tryRestart`/`abort` restart if one is
/// established, else the jump to the top-level context: for an error
/// (`error` is its call and message, which the exit records for embedders,
/// `R_tryEval`'s caller), an error exit to the top level; otherwise a
/// plain jump there.
// R: errors.c:jump_to_top_ex
fn jump_to_top_ex(
    cx: &mut Cx,
    traceback: bool,
    try_user_handler: bool,
    process_warnings: bool,
    reset_console: bool,
    ignore_restart_contexts: bool,
    error: Option<(Option<Root<'_>>, &[u8])>,
) -> RError {
    let st = state(cx);
    let old_in_error = st.in_error.get();
    let mut have_handler = false;
    let restore = |cx: &Cx| state(cx).in_error.set(old_in_error);
    // R_OldCStackLimit == 0: not while a C stack overflow is handled
    if !cx.c_stack_overflow_pending() && try_user_handler && st.in_error.get() < 3 {
        if st.in_error.get() == 0 {
            st.in_error.set(1);
        }
        let r = cx.scope(|cx, s| -> RResult<()> {
            let opt = get_option1(cx, s, b"error")?;
            have_handler = opt.bind(cx).sexptype() != NILSXP;
            if have_handler {
                let t = opt.bind(cx).sexptype();
                if t != LANGSXP && t != EXPRSXP {
                    cx.reprintf(b"invalid option \"error\"\n");
                } else {
                    state(cx).in_error.set(3);
                    let g = s.root(cx, cx.global_env());
                    if t == LANGSXP {
                        let o = s.out(cx);
                        crate::eval(cx, opt, g, o)?;
                    } else {
                        let n = opt.bind(cx).len().unwrap_or(0);
                        // (one root each for the element and its value)
                        let (e, v) = (s.out(cx).root(), s.out(cx).root());
                        for i in 0..n {
                            e.replace_with(cx, |cx| Ok(opt.bind(cx).as_vector::<Ref<'_>>().unwrap().get(i)))?;
                            let o = v.reset(cx);
                            crate::eval(cx, e, g, o)?;
                        }
                    }
                    state(cx).in_error.set(old_in_error);
                }
            }
            Ok(())
        });
        if let Err(e) = r {
            restore(cx);
            return e;
        }
        state(cx).in_error.set(old_in_error);
    }
    // print warnings if there are any left to be printed
    if process_warnings && !cx.collected_warnings().is_empty() {
        print_warnings(cx);
    }
    if reset_console {
        cx.flush_console();
    }
    // jump to a browser/try if one is on the stack
    if !ignore_restart_contexts && let Some(e) = try_jump_to_restart(cx) {
        restore(cx);
        return e;
    }
    // only an interactive session, options(error) or
    // options(catch.script.errors) save the traceback
    let catch_script_errors = cx.scope(|cx, s| {
        get_option1(cx, s, b"catch.script.errors")
            .map(|c| {
                // R_isTRUE
                let c = c.bind(cx);
                c.sexptype() == LGLSXP && c.as_ints() == Some(&[1][..])
            })
            .unwrap_or(false)
    });
    if (have_handler || catch_script_errors || robigo_rtbuiltins::sysutils::interactive(cx))
        && traceback
        && state(cx).in_error.get() < 2
        && state(cx).in_error.get() == old_in_error
    {
        state(cx).in_error.set(2);
        let r = save_traceback(cx);
        state(cx).in_error.set(old_in_error);
        if let Err(e) = r {
            return e;
        }
    }
    restore(cx);
    // R_jumpctxt(R_ToplevelContext, 0, NULL)
    match error {
        Some((call, msg)) => cx.record_error_call(call, msg),
        None => {
            let top = cx.toplevel_context().id();
            cx.jump_to(top, 0, None)
        }
    }
}

/// `R_GetTracebackOnly(skip)`'s walk: the calls of the function and
/// builtin records from the innermost out, the first `skip` left out.
/// GNU R gives a call its record's srcref as an attribute; robigo-rt's
/// records have none yet (`keep.source` is not honored), and one that has
/// is refused rather than dropped.
// R: errors.c:R_GetTracebackOnly
fn traceback_calls<'s>(cx: &mut Cx, s: Scope<'s>, mut skip: usize) -> RResult<Vec<Root<'s>>> {
    let mut v = Vec::new();
    let mut has_srcref = false;
    let mut c = Some(cx.global_context());
    while let Some(r) = c {
        if r.callflag() == ctxt::TOPLEVEL {
            break;
        }
        if r.callflag() & (ctxt::FUNCTION | ctxt::BUILTIN) != 0 {
            if skip > 0 {
                skip -= 1;
            } else {
                has_srcref |= r.srcref().sexptype() != NILSXP;
                v.push(r.call());
            }
        }
        c = r.next();
    }
    let v: Vec<Root<'s>> = v.into_iter().map(|c| s.root(cx, c)).collect();
    if has_srcref {
        return Err(robigo_rtbuiltins::util::not_yet(cx, None, "a traceback of calls with srcrefs"));
    }
    Ok(v)
}

/// `.Traceback`: the calls of the function and builtin records, each
/// duplicated (`R_GetTracebackOnly(0)`), as the symbol's value.
// R: errors.c:R_GetTracebackOnly
fn save_traceback(cx: &mut Cx) -> RResult<()> {
    cx.scope(|cx, s| {
        let calls = traceback_calls(cx, s, 0)?;
        let mut dups = Vec::with_capacity(calls.len());
        for c in &calls {
            let o = s.out(cx);
            let r = o.root();
            // (GNU R duplicates deeply; a shallow copy shares the
            // arguments, which nothing here writes)
            cx.shallow_duplicate(*c, o)?;
            dups.push(r);
        }
        let o = s.out(cx);
        let l = o.root();
        cx.build(o, |cx| {
            let mut l = cx.nil();
            for d in dups.iter().rev() {
                l = cx.cons(d.bind(cx), l)?;
            }
            Ok(l)
        })?;
        // SET_SYMVALUE: past base's locks
        let sy = sym(cx, s, b".Traceback")?;
        cx.set_symvalue(sy, l);
        Ok(())
    })
}

/// `.Internal(traceback(skip))`: the calls of the function and builtin
/// records, the innermost `skip` left out, each deparsed: a pairlist of
/// character vectors (`R_GetTraceback`).
// R: errors.c:do_traceback, R_GetTraceback
pub fn do_traceback(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, args, "traceback", 1)?;
    let skip = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(0))?;
    if skip == i32::MIN || skip < 0 {
        return Err(cx.error("invalid 'skip' value"));
    }
    cx.scope(|cx, s| {
        let calls = traceback_calls(cx, s, skip as usize)?;
        let mut lines = Vec::with_capacity(calls.len());
        for c in &calls {
            // deparse1m(CAR(t), 0, DEFAULTDEPARSE)
            let d = crate::deparse::deparse1(cx, c.bind(cx), false, crate::deparse::DEFAULTDEPARSE)
                .map_err(|m| cx.error(&m))?;
            lines.push(d.lines);
        }
        cx.build(out, |cx| {
            let mut l = cx.nil();
            for d in lines.iter().rev() {
                let mut v = cx.alloc_vector::<Char<'_>>(d.len())?;
                for (i, line) in d.iter().enumerate() {
                    v.set(i, cx.mk_char(line, Encoding::Native)?);
                }
                l = cx.cons(v.finish().as_ref(), l)?;
            }
            Ok(l)
        })
    })
}

/// `try_jump_to_restart()`: an invocation of the innermost `browser`,
/// `tryRestart` or `abort` restart on the restart stack, if there is one.
// R: errors.c:try_jump_to_restart
fn try_jump_to_restart(cx: &mut Cx) -> Option<RError> {
    cx.scope(|cx, s| {
        let mut l = cx.restart_stack();
        while let Some(c) = l.as_cons() {
            let restart = c.car();
            if let Some(v) = restart.as_vector::<Ref<'_>>().filter(|v| restart.sexptype() == VECSXP && v.len() > 1) {
                let name = v.get(0);
                if let Some(n) = name.as_vector::<Char<'_>>().filter(|n| name.sexptype() == STRSXP && n.len() == 1)
                    && matches!(n.get(0).bytes(), b"browser" | b"tryRestart" | b"abort")
                {
                    let r = s.root(cx, restart);
                    let nil = s.root(cx, cx.nil());
                    return Some(invoke_restart(cx, r, nil));
                }
            }
            l = c.cdr();
        }
        None
    })
}

/// `RESTART_EXIT(r)`.
fn restart_exit(r: Ref<'_>) -> Ref<'_> {
    r.as_vector::<Ref<'_>>().expect("robigo: a restart that is not a list").get(1)
}

/// `invokeRestart(r, arglist)`: the restart stack popped to `r`'s entry,
/// then a jump to its frame carrying the arguments (an internal restart: a
/// jump to its context); a restart with no exit is the top level.
// R: errors.c:invokeRestart
fn invoke_restart(cx: &mut Cx, r: Root<'_>, arglist: Root<'_>) -> RError {
    cx.scope(|cx, s| {
        let exit = s.root(cx, restart_exit(r.bind(cx)));
        if exit.bind(cx).sexptype() == NILSXP {
            let nil = s.root(cx, cx.nil());
            cx.set_restart_stack(nil);
            return jump_to_toplevel(cx);
        }
        loop {
            let stack = cx.restart_stack();
            let Some(c) = stack.as_cons() else { break };
            let this = restart_exit(c.car()).sexp();
            let rest = s.root(cx, c.cdr());
            if this == exit.bind(cx).sexp() {
                cx.set_restart_stack(rest);
                if let robigo_rtcore::View::ExtPtr(p) = exit.bind(cx).view() {
                    // R_JumpToContext(c, CTXT_RESTART, R_RestartToken): an
                    // internal restart's exit is its record's address
                    let addr = p.addr() as usize;
                    let Some(target) = context_at(cx, addr) else {
                        return cx.error("target context is not on the stack");
                    };
                    let token = s.root(cx, cx.restart_token());
                    return cx.jump_to_context(target, ctxt::RESTART, Some(token));
                }
                return cx.find_context(ctxt::FUNCTION, exit, Some(arglist));
            }
            cx.set_restart_stack(rest);
        }
        cx.error("restart not on stack")
    })
}

/// The record on the chain at `addr` (an internal restart's exit).
fn context_at(cx: &Cx, addr: usize) -> Option<robigo_rtcore::ContextId> {
    let mut c = Some(cx.global_context());
    while let Some(r) = c {
        if r.id().addr() == addr {
            return Some(r.id());
        }
        c = r.next();
    }
    None
}

/// `checkRestartStacks(cptr)`: an old record (one whose stacks are not the
/// current ones) takes an internal restart only with its restart bit set.
// R: errors.c:checkRestartStacks
fn check_restart_stacks(cx: &mut Cx, c: robigo_rtcore::ContextId) -> RResult<()> {
    let ok = {
        let r = cx.context(c).expect("robigo: a record on the chain");
        (r.handler_stack().sexp() == cx.handler_stack().sexp() && r.restart_stack().sexp() == cx.restart_stack().sexp())
            || r.callflag() & ctxt::RESTART != 0
    };
    if ok { Ok(()) } else { Err(cx.error("handler or restart stack mismatch in old restart")) }
}

/// `addInternalRestart(cptr, cname)`: a restart named `cname` whose exit
/// is the record `c` (an external pointer to it), pushed.
// R: errors.c:addInternalRestart
fn add_internal_restart(cx: &mut Cx, c: robigo_rtcore::ContextId, cname: &[u8]) -> RResult<()> {
    check_restart_stacks(cx, c)?;
    cx.scope(|cx, s| {
        let o = s.out(cx);
        let stack = o.root();
        cx.build(o, |cx| {
            let mut name = cx.alloc_vector::<Char<'_>>(1)?;
            name.set(0, cx.mk_char(cname, Encoding::Native)?);
            let name = name.finish().as_ref();
            let nil = cx.nil();
            let exit = cx.extptr(c.addr(), nil, nil)?;
            let mut entry = cx.alloc_vector::<Ref<'_>>(2)?;
            entry.set(0, name);
            entry.set(1, exit);
            let mut k = cx.alloc_vector::<Char<'_>>(1)?;
            k.set(0, cx.mk_char(b"restart", Encoding::Native)?);
            let attrs = cx.cons_tagged(k.finish().as_ref(), cx.nil(), cx.sym(RSym::Class))?;
            entry.set_attrib(attrs);
            entry.set_object(true);
            cx.cons(entry.finish().as_ref(), cx.restart_stack())
        })?;
        cx.set_restart_stack(stack);
        Ok(())
    })
}

/// `R_InsertRestartHandlers(cptr, cname)`: on the record `c`, an error
/// handler (`options(browser.error.handler)` if a function, else
/// `R_RestartToken`, which sends an error to the default handling) and the
/// internal restart `cname`.
// R: errors.c:R_InsertRestartHandlers
fn insert_restart_handlers(cx: &mut Cx, c: robigo_rtcore::ContextId, cname: &[u8]) -> RResult<()> {
    check_restart_stacks(cx, c)?;
    cx.scope(|cx, s| {
        let opt = get_option1(cx, s, b"browser.error.handler")?;
        let h = if matches!(opt.bind(cx).sexptype(), CLOSXP | BUILTINSXP | SPECIALSXP) {
            opt
        } else {
            s.root(cx, cx.restart_token())
        };
        let rho = s.root(cx, cx.context(c).expect("robigo: a record on the chain").cloenv());
        let o = s.out(cx);
        let stack = o.root();
        cx.build(o, |cx| {
            // mkHandlerEntry(mkChar("error"), rho, h, rho, R_NilValue, TRUE)
            let mut e = cx.alloc_vector::<Ref<'_>>(5)?;
            e.set(0, cx.mk_char(b"error", Encoding::Native)?.as_ref());
            e.set(1, rho.bind(cx));
            e.set(2, h.bind(cx));
            e.set(3, rho.bind(cx));
            e.set(4, cx.nil());
            let e = e.finish().as_ref();
            // SAFETY: fresh; gp is a header field (SETLEVELS: calling).
            unsafe { e.sexp().set_gp(1) };
            cx.cons(e, cx.handler_stack())
        })?;
        cx.set_handler_stack(stack);
        Ok(())
    })?;
    add_internal_restart(cx, c, cname)
}

/// `jump_to_toplevel()`: no traceback, no `options(error)`, warnings
/// printed, restarts ignored: the jump to the top-level context.
// R: errors.c:jump_to_toplevel
fn jump_to_toplevel(cx: &mut Cx) -> RError {
    jump_to_top_ex(cx, false, false, true, true, true, None)
}

// ---- warnings -------------------------------------------------------------------

/// The isolate's warning hook: GNU R's `warningcall(call, "%s", msg)`
/// (`vsignalWarning`): `.signalSimpleWarning(msg, base::quote(call))` in
/// base when base defines it, else the default handling.
// R: errors.c:vsignalWarning
pub fn warning_hook(cx: &mut Cx, call: Option<Root<'_>>, msg: &[u8]) -> RResult<()> {
    let msg = robigo_rtcore::c_str_bytes(msg);
    let hooked = cx.scope(|cx, s| -> RResult<bool> {
        let hooksym = sym(cx, s, b".signalSimpleWarning")?;
        let quote = sym(cx, s, b"quote")?;
        if !base_bound(cx, hooksym) || !base_bound(cx, quote) {
            return Ok(false);
        }
        let qcall = base_quote(cx, s, call)?;
        let m = mk_string(cx, s, trunc(msg, BUFSIZE - 2))?;
        let o = s.out(cx);
        let hcall = o.root();
        cx.build(o, |cx| {
            let nil = cx.nil();
            let a = cx.cons(qcall.bind(cx), nil)?;
            let a = cx.cons(m.bind(cx), a)?;
            cx.lcons(hooksym.bind(cx), a)
        })?;
        let base = s.root(cx, cx.base_env());
        eval_keep_vis(cx, hcall, base)?;
        Ok(true)
    })?;
    if hooked {
        return Ok(());
    }
    vwarningcall_dflt(cx, call, msg)
}

/// `SYMVALUE(sym) != R_UnboundValue`: `sym` is bound in base.
fn base_bound(cx: &Cx, sym: Root<'_>) -> bool {
    let s = sym.bind(cx).as_sym().unwrap();
    let robigo_rtcore::View::Env(base) = cx.base_env().view() else { unreachable!() };
    !matches!(cx.frame_binding(base, s), robigo_rtcore::Lookup::Unbound)
}

/// `vwarningcall_dflt(call, "%s", msg)`: under `options(warn)`, collected
/// (0), printed at once (1), or turned into an error (2 and above).
// R: errors.c:vwarningcall_dflt
pub fn vwarningcall_dflt(cx: &mut Cx, call: Option<Root<'_>>, msg: &[u8]) -> RResult<()> {
    if state(cx).in_warning.get() {
        return Ok(());
    }
    cx.scope(|cx, s| {
        let expr = get_option1(cx, s, b"warning.expression")?;
        if expr.bind(cx).sexptype() != NILSXP {
            let t = expr.bind(cx).sexptype();
            if t != LANGSXP && t != EXPRSXP {
                return Err(cx.error("invalid option \"warning.expression\""));
            }
            // the innermost function record's environment
            let env = {
                let mut c = cx.global_context();
                while c.callflag() & ctxt::FUNCTION == 0 && c.callflag() != 0 {
                    c = c.next().unwrap();
                }
                c.cloenv()
            };
            let env = s.root(cx, env);
            return eval_keep_vis(cx, expr, env);
        }
        let warn = get_option1(cx, s, b"warn")?;
        let mut w = as_integer_opt(warn.bind(cx)).unwrap_or(0);
        let st = state(cx);
        if w <= 0 && st.immediate_warning.get() {
            w = 1;
        }
        if w < 0 || st.in_warning.get() || st.in_error.get() != 0 {
            return Ok(());
        }
        let psize = BUFSIZE.min(cx.report_options().warn_length.get() + 1);
        let buf = print_trunc(msg, psize);
        st.in_warning.set(true);
        let r = (|| -> RResult<()> {
            if w >= 2 {
                state(cx).in_warning.set(false);
                let mut m = b"(converted from warning) ".to_vec();
                m.extend_from_slice(&buf);
                // the buffer's bytes, as errorcall gets them
                return Err(cx.error_call_bytes(call, &m));
            } else if w == 1 {
                if let Some(c) = call {
                    force_call_promises(cx, c)?;
                }
                let callr = call.map(|c| c.bind(cx)).filter(|c| c.sexptype() != NILSXP);
                let dc = callr.map(|c| dcall(cx, c)).unwrap_or_default();
                let mut out = Vec::new();
                if dc.is_empty() {
                    out.extend_from_slice(b"Warning:");
                } else {
                    out.extend_from_slice(b"Warning in ");
                    out.extend_from_slice(&dc);
                    out.extend_from_slice(b" :");
                    if !(state(cx).no_break_warning.get() || 18 + wd(&dc) + wd(line1(&buf)) <= LONGWARN) {
                        out.extend_from_slice(b"\n ");
                    }
                }
                out.push(b' ');
                out.extend_from_slice(&buf);
                out.push(b'\n');
                if cx.report_options().show_warn_calls.get()
                    && let Some(c) = callr
                {
                    let tr = concise_traceback(cx, Some(c), 0);
                    if !tr.is_empty() {
                        out.extend_from_slice(b"Calls: ");
                        out.extend_from_slice(&tr);
                        out.push(b'\n');
                    }
                }
                cx.reprintf(&out);
            } else {
                let mut buf = buf;
                if cx.report_options().show_warn_calls.get()
                    && let Some(c) = call.map(|c| c.bind(cx)).filter(|c| c.sexptype() != NILSXP)
                {
                    let tr = concise_traceback(cx, Some(c), 0);
                    if !tr.is_empty() && tr.len() + buf.len() + 8 < BUFSIZE {
                        buf.extend_from_slice(b"\nCalls: ");
                        buf.extend_from_slice(&tr);
                    }
                }
                cx.collect_warning(call, &buf)?;
            }
            Ok(())
        })();
        state(cx).in_warning.set(false);
        r
    })
}

/// A message formatted into a buffer of `psize` (`Rvsnprintf_mbcs`), with
/// `RprintTrunc`'s " [... truncated]" when it did not fit.
// R: errors.c:RprintTrunc
fn print_trunc(msg: &[u8], psize: usize) -> Vec<u8> {
    let mut b = trunc(msg, psize.saturating_sub(1)).to_vec();
    if msg.len() >= psize {
        let t = b"[... truncated]";
        if b.len() + 1 + t.len() < BUFSIZE {
            b.push(b' ');
            b.extend_from_slice(t);
        }
    }
    b
}

/// `PrintWarnings()`: the deferred warnings written ("Warning message:",
/// "Warning messages:", or a count), saved as `last.warning`, the list
/// cleared.
// R: errors.c:PrintWarnings
pub fn print_warnings(cx: &mut Cx) {
    let n = cx.collected_warnings().len();
    if n == 0 {
        return;
    }
    let st = state(cx);
    if st.in_print_warnings.get() {
        cx.clear_warnings();
        cx.reprintf(b"Lost warning messages\n");
        return;
    }
    st.in_print_warnings.set(true);
    let mut out: Vec<u8> = Vec::new();
    {
        let ws = cx.collected_warnings();
        // `whole`: the one-warning branch in an MBCS locale measures all of
        // `msg`, not its first line (it takes `wd(msg)` where it meant `msg1`).
        let item = |out: &mut Vec<u8>, call: Ref<'_>, msg: &[u8], indent: usize, whole: bool| {
            if call.sexptype() == NILSXP {
                out.extend_from_slice(msg);
                out.extend_from_slice(b" \n");
                return;
            }
            let dc = dcall(cx, call);
            out.extend_from_slice(b"In ");
            out.extend_from_slice(&dc);
            out.extend_from_slice(b" :");
            let msgline1 = if whole { wd(msg) } else { wd(line1(msg)) };
            if indent + wd(&dc) + msgline1 > LONGWARN {
                out.extend_from_slice(b"\n ");
            }
            out.push(b' ');
            out.extend_from_slice(msg);
            out.push(b'\n');
        };
        if n == 1 {
            out.extend_from_slice(b"Warning message:\n");
            let (c, m) = ws[0];
            item(&mut out, c, m.bytes(), 6, cx.locale().utf8);
        } else if n <= 10 {
            out.extend_from_slice(b"Warning messages:\n");
            for (i, &(c, m)) in ws.iter().enumerate() {
                out.extend_from_slice(format!("{}: ", i + 1).as_bytes());
                item(&mut out, c, m.bytes(), 10, false);
            }
        } else if n < cx.nwarnings() {
            out.extend_from_slice(format!("There were {n} warnings (use warnings() to see them)\n").as_bytes());
        } else {
            let m = cx.nwarnings();
            out.extend_from_slice(
                format!("There were {m} or more warnings (use warnings() to see the first {m})\n").as_bytes(),
            );
        }
    }
    cx.reprintf(&out);
    // now truncate and install last.warning
    let _ = save_last_warning(cx);
    state(cx).in_print_warnings.set(false);
    cx.clear_warnings();
}

/// `last.warning`: a list of the warnings' calls named by their messages,
/// in base.
fn save_last_warning(cx: &mut Cx) -> RResult<()> {
    cx.scope(|cx, s| {
        let o = s.out(cx);
        let v = o.root();
        cx.build(o, |cx| {
            let ws = cx.collected_warnings();
            let mut names = cx.alloc_vector::<Char<'_>>(ws.len())?;
            for (i, &(_, m)) in ws.iter().enumerate() {
                names.set(i, m);
            }
            let names = names.finish();
            let mut l = cx.alloc_vector::<Ref<'_>>(ws.len())?;
            for (i, &(c, _)) in ws.iter().enumerate() {
                l.set(i, c);
            }
            let nsym = cx.sym(RSym::Names);
            let attrs = cx.cons_tagged(names.as_ref(), cx.nil(), nsym)?;
            l.set_attrib(attrs);
            Ok(l.finish().into())
        })?;
        // SET_SYMVALUE: past base's locks
        let sy = sym(cx, s, b"last.warning")?;
        cx.set_symvalue(sy, v);
        Ok(())
    })
}

// ---- the .Internals ---------------------------------------------------------------

/// `checkArity(op, args)` of one of these `.Internal`s
/// (`crate::prims::check_arity`: `error()`'s, against the context's call).
fn check_arity(cx: &mut Cx, call: Root<'_>, args: Args<'_>, name: &str, n: usize) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), name, n, true)
}

/// `asLogical(x)` as `do_stop`/`do_warning` read their flags: TRUE, FALSE
/// or NA (None).
fn as_logical(cx: &Cx, x: Ref<'_>) -> Option<bool> {
    match robigo_rtbuiltins::util::as_logical(cx, x) {
        i32::MIN => None,
        v => Some(v != 0),
    }
}

/// `findCall()`: the call of the first function record below the
/// innermost (the closure that called `stop` or `warning`), R_NilValue at
/// the top level.
// R: errors.c:findCall
fn find_call(cx: &Cx) -> Ref<'_> {
    let mut c = cx.global_context().next();
    while let Some(r) = c {
        if r.callflag() == ctxt::TOPLEVEL {
            break;
        }
        if r.callflag() & ctxt::FUNCTION != 0 {
            return r.call();
        }
        c = r.next();
    }
    cx.nil()
}

/// The message argument of `stop`/`warning`: `coerceVector(x, STRSXP)`'s
/// first element translated (`translateChar`), None when it is not a valid
/// string (`isValidString`).
// R: errors.c:do_stop, do_warning
fn message_arg(cx: &mut Cx, x: Root<'_>) -> RResult<Option<Vec<u8>>> {
    let v = x.bind(cx);
    if v.sexptype() == STRSXP {
        if v.len() == Some(0) {
            return Ok(None);
        }
        return robigo_rtbuiltins::text::native_elt(cx, x, 0).map(Some);
    }
    let r = match robigo_rtbuiltins::coerce::Strings::of(cx, v) {
        Ok(s) if !s.is_empty() => robigo_rtbuiltins::text::to_native(s.get_or_na(0), cx.locale())
            .map(|b| Some(b.into_owned()))
            .map_err(|e| e.to_string()),
        Ok(_) => Ok(None),
        Err(e) => Err(e.message()),
    };
    r.map_err(|m| cx.error(&m))
}

/// `.Internal(stop(call., message))`.
// R: errors.c:do_stop
pub fn do_stop(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, _out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, args, "stop", 2)?;
    cx.scope(|cx, s| {
        let c_call = if as_logical(cx, args.get(cx, 0)) == Some(true) { find_call(cx) } else { cx.nil() };
        let c_call = s.root(cx, c_call);
        let c = (c_call.bind(cx).sexptype() != NILSXP).then_some(c_call);
        let m = args.root(1);
        if m.bind(cx).sexptype() != NILSXP {
            match message_arg(cx, m)? {
                Some(msg) => Err(cx.error_call_bytes(c, &msg)),
                None => Err(cx.error_call(c, " [invalid string in stop(.)]")),
            }
        } else {
            Err(cx.error_call(c, ""))
        }
    })
}

/// `.Internal(warning(call., immediate., noBreaks., message))`: the
/// message, invisibly.
// R: errors.c:do_warning
pub fn do_warning(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, args, "warning", 4)?;
    cx.scope(|cx, s| {
        let c_call = if as_logical(cx, args.get(cx, 0)) == Some(true) { find_call(cx) } else { cx.nil() };
        let c_call = s.root(cx, c_call);
        let c = (c_call.bind(cx).sexptype() != NILSXP).then_some(c_call);
        state(cx).immediate_warning.set(as_logical(cx, args.get(cx, 1)) == Some(true));
        state(cx).no_break_warning.set(as_logical(cx, args.get(cx, 2)) == Some(true));
        let m = args.root(3);
        let r = if m.bind(cx).sexptype() != NILSXP {
            match message_arg(cx, m) {
                Ok(Some(msg)) => cx.warning_call_bytes(c, &msg),
                Ok(None) => cx.warning_call(c, " [invalid string in warning(.)]"),
                Err(e) => Err(e),
            }
        } else {
            cx.warning_call(c, "")
        };
        // reset to internal calls
        state(cx).immediate_warning.set(false);
        state(cx).no_break_warning.set(false);
        r?;
        // (the coerced message: a character vector stays itself)
        out.set(cx, m.bind(cx));
        Ok(())
    })
}

/// `.Internal(geterrmessage())`.
// R: errors.c:do_geterrmessage
pub fn do_geterrmessage(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, args, "geterrmessage", 0)?;
    let b = state(cx).errbuf.borrow().clone();
    cx.build(out, |cx| {
        let mut v = cx.alloc_vector::<Char<'_>>(1)?;
        v.set(0, cx.mk_char(&b, Encoding::Native)?);
        Ok(v.finish().into())
    })
}

/// `.Internal(seterrmessage(msg))`.
// R: errors.c:do_seterrmessage
pub fn do_seterrmessage(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, args, "seterrmessage", 1)?;
    let m = args.get(cx, 0);
    let ok = m.sexptype() == STRSXP && m.len() == Some(1);
    if !ok {
        return Err(cx.error("error message must be a character string"));
    }
    let b = m.as_vector::<Char<'_>>().unwrap().get(0).bytes().to_vec();
    *state(cx).errbuf.borrow_mut() = trunc(&b, BUFSIZE - 1).to_vec();
    out.set(cx, cx.nil());
    Ok(())
}

/// `.Internal(printDeferredWarnings())`.
// R: errors.c:do_printDeferredWarnings
pub fn do_print_deferred_warnings(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    check_arity(cx, call, args, "printDeferredWarnings", 0)?;
    if cx.report_options().show_error_messages.get() && !cx.collected_warnings().is_empty() {
        cx.reprintf(b"In addition: ");
        print_warnings(cx);
    }
    out.set(cx, cx.nil());
    Ok(())
}

/// `.Internal(.addCondHands(classes, handlers, parentenv, target,
/// calling))`: one entry per class pushed (the first class innermost),
/// sharing one result vector; the old stack returned.
// R: errors.c:do_addCondHands
pub fn do_add_cond_hands(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, args, ".addCondHands", 5)?;
    let (classes, handlers) = (args.get(cx, 0), args.get(cx, 1));
    if classes.sexptype() == NILSXP || handlers.sexptype() == NILSXP {
        out.set(cx, cx.handler_stack());
        return Ok(());
    }
    if classes.sexptype() != STRSXP || handlers.sexptype() != VECSXP || classes.len() != handlers.len() {
        return Err(cx.error("bad handler data"));
    }
    let calling = robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 4));
    let n = handlers.len().unwrap();
    cx.scope(|cx, s| {
        let old = s.root(cx, cx.handler_stack());
        let o = s.out(cx);
        let result = o.root();
        cx.build(o, |cx| {
            let mut r = cx.alloc_vector::<Ref<'_>>(RESULT_SIZE)?;
            r.set(RESULT_SIZE - 1, state(cx).handler_result_token.bind(cx));
            Ok(r.finish().into())
        })?;
        let o = s.out(cx);
        let stack = o.root();
        cx.build(o, |cx| {
            let classes = args.get(cx, 0).as_vector::<Char<'_>>().unwrap();
            let handlers = args.get(cx, 1).as_vector::<Ref<'_>>().unwrap();
            let mut newstack = old.bind(cx);
            for i in (0..n).rev() {
                // R: errors.c:mkHandlerEntry
                let mut e = cx.alloc_vector::<Ref<'_>>(5)?;
                e.set(0, classes.get(i).as_ref());
                e.set(1, args.get(cx, 2));
                e.set(2, handlers.get(i));
                e.set(3, args.get(cx, 3));
                e.set(4, result.bind(cx));
                let e = e.finish().as_ref();
                // SAFETY: fresh; gp is a header field (SETLEVELS).
                unsafe { e.sexp().set_gp(calling as u16) };
                newstack = cx.cons(e, newstack)?;
            }
            Ok(newstack)
        })?;
        cx.set_handler_stack(stack);
        out.set(cx, old.bind(cx));
        Ok(())
    })
}

/// `.Internal(.addGlobHands(classes, handlers, parentenv, target,
/// calling))` (`globalCallingHandlers`): with no handlers established
/// below the top level, the global handlers made anew from these (as
/// `.addCondHands` makes entries, on an empty stack), and every record
/// down to the top level given them.
// R: errors.c:do_addGlobHands
pub fn do_add_glob_hands(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, args, ".addGlobHands", 5)?;
    // check for handlers on the stack before proceeding (PR1826)
    if !cx.handlers_only_global() {
        return Err(cx.error("should not be called with handlers on the stack"));
    }
    cx.scope(|cx, s| {
        let nil = s.root(cx, cx.nil());
        cx.set_handler_stack(nil);
        let o = s.out(cx);
        do_add_cond_hands(cx, call, args, env, o)?;
        let stack = s.root(cx, cx.handler_stack());
        cx.set_global_handlers(stack);
        Ok(())
    })?;
    out.set(cx, cx.nil());
    Ok(())
}

/// `.Internal(.resetCondHands(stack))`.
// R: errors.c:do_resetCondHands
pub fn do_reset_cond_hands(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, args, ".resetCondHands", 1)?;
    cx.set_handler_stack(args.root(0));
    out.set(cx, cx.nil());
    Ok(())
}

/// `findConditionHandler(cond)`: the handler stack's first entry whose
/// class is one of the condition's classes.
// R: errors.c:findConditionHandler
fn find_condition_handler<'g>(cx: &'g Cx, cond: Ref<'g>) -> Option<Ref<'g>> {
    let classes = robigo_rtbuiltins::attrib::get_value(cond, b"class")?;
    let classes = classes.as_vector::<Char<'_>>().filter(|_| classes.sexptype() == STRSXP)?;
    let names: Vec<&[u8]> = (0..classes.len()).map(|i| classes.get(i).bytes()).collect();
    find_handler(cx, |c| names.contains(&c))
}

/// `.Internal(.signalCondition(cond, message, call))`: each handler for
/// the condition's classes, innermost first (each popped while it runs):
/// a calling handler is called with the condition (in the global
/// environment), an exiting one is a jump; the stack restored after.
// R: errors.c:do_signalCondition
pub fn do_signal_condition(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, args, ".signalCondition", 3)?;
    let (cond, msg, ecall) = (args.root(0), args.root(1), args.root(2));
    cx.scope(|cx, s| {
        let oldstack = s.root(cx, cx.handler_stack());
        while let Some(list) = find_condition_handler(cx, cond.bind(cx)) {
            let list = s.root(cx, list);
            let entry = s.root(cx, list.bind(cx).as_cons().unwrap().car());
            pop_handlers_to(cx, list);
            if is_calling(entry.bind(cx)) {
                let h = entry_elt(entry.bind(cx), entry::HANDLER);
                if h.sexp() == cx.restart_token().sexp() {
                    let m = msg.bind(cx);
                    let text = match m.as_vector::<Char<'_>>().filter(|v| m.sexptype() == STRSXP && !v.is_empty()) {
                        Some(v) => robigo_rtbuiltins::coerce::text(v.get(0)).bytes().to_vec(),
                        None => return Err(cx.error("error message not a string")),
                    };
                    let ec = (ecall.bind(cx).sexptype() != NILSXP).then_some(ecall);
                    return Err(verrorcall_dflt(cx, ec, &text));
                }
                let h = s.root(cx, h);
                let o = s.out(cx);
                let hcall = o.root();
                cx.build(o, |cx| {
                    let a = cx.cons(cond.bind(cx), cx.nil())?;
                    cx.lcons(h.bind(cx), a)
                })?;
                let g = s.root(cx, cx.global_env());
                let o = s.out(cx);
                crate::eval(cx, hcall, g, o)?;
            } else {
                let ec = (ecall.bind(cx).sexptype() != NILSXP).then_some(ecall);
                return Err(goto_exiting_handler(cx, cond, ec, entry));
            }
        }
        cx.set_handler_stack(oldstack);
        out.set(cx, cx.nil());
        Ok(())
    })
}

/// `.Internal(.dfltWarn(message, call))`.
// R: errors.c:do_dfltWarn
pub fn do_dflt_warn(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, args, ".dfltWarn", 2)?;
    let m = args.get(cx, 0);
    if m.sexptype() != STRSXP || m.len() != Some(1) {
        return Err(cx.error("bad error message"));
    }
    let msg = robigo_rtbuiltins::coerce::text(m.as_vector::<Char<'_>>().unwrap().get(0)).bytes().to_vec();
    let ecall = args.root(1);
    let ec = (ecall.bind(cx).sexptype() != NILSXP).then_some(ecall);
    vwarningcall_dflt(cx, ec, &msg)?;
    out.set(cx, cx.nil());
    Ok(())
}

/// `.Internal(.dfltStop(message, call))`.
// R: errors.c:do_dfltStop
pub fn do_dflt_stop(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, _out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, args, ".dfltStop", 2)?;
    let m = args.get(cx, 0);
    if m.sexptype() != STRSXP || m.len() != Some(1) {
        return Err(cx.error("bad error message"));
    }
    let msg = robigo_rtbuiltins::coerce::text(m.as_vector::<Char<'_>>().unwrap().get(0)).bytes().to_vec();
    let ecall = args.root(1);
    let ec = (ecall.bind(cx).sexptype() != NILSXP).then_some(ecall);
    Err(verrorcall_dflt(cx, ec, &msg))
}

/// `.Internal(.getRestart(i))`: the `i`th restart on the stack, the
/// `abort` restart one past its end, NULL after.
// R: errors.c:do_getRestart
pub fn do_get_restart(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, args, ".getRestart", 1)?;
    let mut i = as_integer_opt(args.get(cx, 0)).unwrap_or(i32::MIN);
    let mut list = cx.restart_stack();
    while let Some(c) = list.as_cons() {
        if i <= 1 {
            break;
        }
        list = c.cdr();
        i -= 1;
    }
    if let Some(c) = list.as_cons() {
        out.set(cx, c.car());
        return Ok(());
    }
    if i == 1 {
        return cx.build(out, |cx| {
            let mut name = cx.alloc_vector::<Char<'_>>(1)?;
            name.set(0, cx.mk_char(b"abort", Encoding::Native)?);
            let name = name.finish().as_ref();
            let mut entry = cx.alloc_vector::<Ref<'_>>(2)?;
            entry.set(0, name);
            entry.set(1, cx.nil());
            let mut k = cx.alloc_vector::<Char<'_>>(1)?;
            k.set(0, cx.mk_char(b"restart", Encoding::Native)?);
            let csym = cx.sym(RSym::Class);
            let attrs = cx.cons_tagged(k.finish().as_ref(), cx.nil(), csym)?;
            entry.set_attrib(attrs);
            entry.set_object(true);
            Ok(entry.finish().into())
        });
    }
    out.set(cx, cx.nil());
    Ok(())
}

/// `CHECK_RESTART(r)`.
fn check_restart(cx: &mut Cx, r: Root<'_>) -> RResult<()> {
    let r = r.bind(cx);
    if r.sexptype() != VECSXP || r.len().unwrap_or(0) < 2 {
        return Err(cx.error("bad restart"));
    }
    Ok(())
}

/// `.Internal(.addRestart(restart))`.
// R: errors.c:do_addRestart
pub fn do_add_restart(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, args, ".addRestart", 1)?;
    check_restart(cx, args.root(0))?;
    cx.scope(|cx, s| {
        let o = s.out(cx);
        let st = o.root();
        cx.build(o, |cx| cx.cons(args.get(cx, 0), cx.restart_stack()))?;
        cx.set_restart_stack(st);
        Ok(())
    })?;
    out.set(cx, cx.nil());
    Ok(())
}

/// `.Internal(.addTryHandlers())`: the calling function's record takes a
/// `tryRestart` (its restart bit set): an error the handlers do not catch
/// is reported, then the function returns `R_RestartToken`.
// R: errors.c:do_addTryHandlers
pub fn do_add_try_handlers(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, args, ".addTryHandlers", 0)?;
    let g = cx.global_context();
    if g.callflag() == ctxt::TOPLEVEL || g.callflag() & ctxt::FUNCTION == 0 {
        return Err(cx.error("not in a try context"));
    }
    let (id, flag) = (g.id(), g.callflag());
    // SET_RESTART_BIT_ON(R_GlobalContext->callflag)
    cx.set_context_callflag(id, flag | ctxt::RESTART);
    insert_restart_handlers(cx, id, b"tryRestart")?;
    out.set(cx, cx.nil());
    Ok(())
}

/// `.Internal(.invokeRestart(r, args))`.
// R: errors.c:do_invokeRestart
pub fn do_invoke_restart(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, _out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, args, ".invokeRestart", 2)?;
    check_restart(cx, args.root(0))?;
    Err(invoke_restart(cx, args.root(0), args.root(1)))
}

/// `.Internal(interruptsSuspended(susp))`: the flag before, set from the
/// argument if one is given.
// R: errors.c:do_interruptsSuspended
pub fn do_interrupts_suspended(
    cx: &mut Cx,
    _call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let orig = cx.interrupts_suspended();
    if !args.is_empty() {
        let v = robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 0));
        if v == i32::MIN {
            return Err(cx.error("invalid 'susp' argument"));
        }
        cx.set_interrupts_suspended(v != 0);
    }
    cx.build_scalar(out, robigo_rtcore::Lgl(orig as i32))
}

// ---- embedders (A1) ---------------------------------------------------------------

/// `R_tryEval(e, env, &errorOccurred)`: `eval(e, env)` in a top-level
/// context, the error (reported as GNU R reports it) turned into `None`;
/// `R_tryEvalSilent` with `silent` (no report: `R_ShowErrorMessages`
/// off). `R_Visible` is kept.
// R: context.c:R_tryEval, R_tryEvalSilent
pub fn try_eval(cx: &mut Cx, e: Root<'_>, env: Root<'_>, silent: bool, out: Out<'_>) -> RResult<bool> {
    let oldshow = cx.report_options().show_error_messages.get();
    if silent {
        cx.report_options().show_error_messages.set(false);
    }
    let r = cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        let b = s.root(cx, cx.base_env());
        let o = out.root();
        cx.toplevel(g, b, |cx| {
            let slot = o.reset(cx);
            crate::eval(cx, e, env, slot)
        })
    });
    cx.report_options().show_error_messages.set(oldshow);
    r
}

// ---- C-level tryCatch (R_tryCatch, R_withCallingErrorHandler) ---------------

/// errors.c's `trycatch_callback_source`, verbatim.
const TRYCATCH_CALLBACK: &str = "function(addr, classes, fin) {\n\
    handler <- function(cond)\n\
        .Internal(C_tryCatchHelper(addr, 1L, cond))\n\
    handlers <- rep_len(alist(handler), length(classes))\n\
    names(handlers) <- classes\n\
    if (fin)\n\
\t     handlers <- c(handlers,\n\
            alist(finally = .Internal(C_tryCatchHelper(addr, 2L))))\n\
    args <- c(alist(.Internal(C_tryCatchHelper(addr, 0L))), handlers)\n\
    do.call('tryCatch', args)\n\
}";

/// errors.c's `wceh_callback_source`, verbatim.
const WCEH_CALLBACK: &str = "function(cond) .Internal(C_tryCatchHelper(addr, 1L, cond))";

/// `R_ParseEvalString(src, R_BaseNamespace)` of the callback `which` (0:
/// `trycatch_callback`, 1: `wceh_callback`), made once per isolate.
// R: errors.c:R_tryCatch, R_withCallingErrorHandler; util.c:R_ParseEvalString
fn c_callback<'s>(cx: &mut Cx, s: Scope<'s>, which: usize) -> RResult<Root<'s>> {
    if let Some(g) = &state(cx).c_callbacks.borrow()[which] {
        return Ok(s.root(cx, g.bind(cx)));
    }
    let src = [TRYCATCH_CALLBACK, WCEH_CALLBACK][which];
    let o = s.out(cx);
    let exprs = o.root();
    let (status, _) = crate::reader::parse_bytes(cx, src.as_bytes(), -1, o)?;
    if status != crate::reader::ParseStatus::Ok {
        return Err(cx.error("robigo: errors.c's C tryCatch callback does not parse"));
    }
    let e = s.root(cx, exprs.bind(cx).as_vector::<Ref<'_>>().expect("robigo: an expression vector").get(0));
    let ns = s.root(cx, cx.base_namespace());
    let o = s.out(cx);
    let f = o.root();
    crate::eval(cx, e, ns, o)?;
    let g = cx.global(f.bind(cx));
    state(cx).c_callbacks.borrow_mut()[which] = Some(g);
    Ok(f)
}

/// `R_tryCatch`'s call into R: `evalKeepVis(lang4(trycatch_callback,
/// addr, conds, fin), R_GlobalEnv)` into `out`, where `addr` is the
/// external pointer to C's `tryCatchData_t` that
/// `.Internal(C_tryCatchHelper())` reads.
// R: errors.c:R_tryCatch
pub fn eval_trycatch_callback(cx: &mut Cx, addr: Root<'_>, conds: Root<'_>, fin: bool, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let cb = c_callback(cx, s, 0)?;
        let o = s.out(cx);
        let call = o.root();
        cx.build(o, |cx| {
            let fin = cx.alloc_with(1, |_| robigo_rtcore::Lgl(fin as i32))?.finish().as_ref();
            let a = cx.cons(fin, cx.nil())?;
            let a = cx.cons(conds.bind(cx), a)?;
            let a = cx.cons(addr.bind(cx), a)?;
            cx.lcons(cb.bind(cx), a)
        })?;
        let g = s.root(cx, cx.global_env());
        // evalKeepVis
        let vis = cx.visible();
        let r = crate::eval(cx, call, g, out);
        cx.set_visible(vis);
        r
    })
}

/// `R_withCallingErrorHandler`'s handler pushed: `wceh_callback` with a
/// new environment holding `addr` (enclosed by the base namespace) as its
/// own, made a calling handler for `"error"` (`mkHandlerEntry(wceh_class,
/// R_GlobalEnv, h, R_NilValue, R_NilValue, TRUE)`) on top of
/// `R_HandlerStack`. The old stack is the result, for C to put back after
/// the body.
// R: errors.c:R_withCallingErrorHandler
pub fn push_calling_error_handler<'s>(cx: &mut Cx, s: Scope<'s>, addr: Root<'_>) -> RResult<Root<'s>> {
    let cb = c_callback(cx, s, 1)?;
    let old = s.root(cx, cx.handler_stack());
    let addr_sym = sym(cx, s, b"addr")?;
    let o = s.out(cx);
    let env = o.root();
    let ns = s.root(cx, cx.base_namespace());
    cx.build(o, |cx| cx.new_env(ns.bind(cx), false, 1))?;
    cx.define_var(addr_sym, addr, env)?;
    let o = s.out(cx);
    let stack = o.root();
    cx.build(o, |cx| {
        let robigo_rtcore::View::Closure(f) = cb.bind(cx).view() else {
            unreachable!("robigo: wceh_callback is a closure")
        };
        // duplicate(wceh_callback), SET_CLOENV(h, env)
        let h = cx.closure(f.formals(), f.body(), env.bind(cx))?;
        let mut e = cx.alloc_vector::<Ref<'_>>(5)?;
        e.set(0, cx.mk_char(b"error", Encoding::Native)?.as_ref());
        e.set(1, cx.global_env());
        e.set(2, h);
        e.set(3, cx.nil());
        e.set(4, cx.nil());
        let e = e.finish().as_ref();
        // SAFETY: fresh; gp is a header field (SETLEVELS: calling).
        unsafe { e.sexp().set_gp(1) };
        cx.cons(e, old.bind(cx))
    })?;
    cx.set_handler_stack(stack);
    Ok(old)
}

// ---- user interrupts ---------------------------------------------------------

/// The isolate's interrupt hook, GNU R's `onintr()` (`onintrEx(TRUE)`): the
/// `interrupt` condition signalled in a restart context that offers the
/// internal restart `resume`; a handler that invokes it lets evaluation go
/// on (`Ok`). Otherwise the console's newline, then the jump to the top
/// level, which `options(error)` sees only while `options(interrupt)` is
/// NULL ("as a transition").
// R: errors.c:onintrEx
pub fn onintr_hook(cx: &mut Cx) -> RResult<()> {
    let resumed = cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        let b = s.root(cx, cx.base_env());
        cx.restart_context(g, b, |cx, c| {
            add_internal_restart(cx, c, b"resume")?;
            signal_interrupt(cx)
        })
    })?;
    if resumed {
        // R_ReturnedValue = R_NilValue; R_Visible = FALSE
        cx.set_visible(false);
        return Ok(());
    }
    let try_user_error = cx.scope(|cx, s| -> RResult<bool> {
        let h = get_option1(cx, s, b"interrupt")?;
        Ok(h.bind(cx).sexptype() == NILSXP)
    })?;
    cx.reprintf(b"\n");
    Err(jump_to_top_ex(cx, true, try_user_error, true, true, false, None))
}

/// `onintrNoResume()` (`onintrEx(FALSE)`), the console read's interrupt
/// (sys-std.c's `handleInterrupt`): the `interrupt` condition signalled with
/// no `resume` restart, then the console's newline and the jump to the top
/// level, as [`onintr_hook`]'s.
// R: errors.c:onintrEx, unix/sys-std.c:handleInterrupt
pub fn onintr_no_resume(cx: &mut Cx) -> RError {
    if let Err(e) = signal_interrupt(cx) {
        return e;
    }
    let try_user_error = match cx.scope(|cx, s| -> RResult<bool> {
        let h = get_option1(cx, s, b"interrupt")?;
        Ok(h.bind(cx).sexptype() == NILSXP)
    }) {
        Ok(b) => b,
        Err(e) => return e,
    };
    cx.reprintf(b"\n");
    jump_to_top_ex(cx, true, try_user_error, true, true, false, None)
}

/// `signalInterrupt()`: the handlers for `interrupt` or `condition`,
/// innermost first, each popped as it runs. A calling handler is called
/// with a new interrupt condition in the global environment; an exiting
/// one is a jump with it (no call). The stack is put back after them, then
/// `options(interrupt)` is called.
// R: errors.c:signalInterrupt, findInterruptHandler
fn signal_interrupt(cx: &mut Cx) -> RResult<()> {
    cx.scope(|cx, s| {
        let oldstack = s.root(cx, cx.handler_stack());
        let g = s.root(cx, cx.global_env());
        while let Some(list) = find_handler(cx, |c| c == b"interrupt" || c == b"condition") {
            let list = s.root(cx, list);
            let entry = s.root(cx, list.bind(cx).as_cons().unwrap().car());
            pop_handlers_to(cx, list);
            let cond = interrupt_condition(cx, s)?;
            if !is_calling(entry.bind(cx)) {
                return Err(goto_exiting_handler(cx, cond, None, entry));
            }
            let h = s.root(cx, entry_elt(entry.bind(cx), entry::HANDLER));
            let o = s.out(cx);
            let hcall = o.root();
            cx.build(o, |cx| {
                let a = cx.cons(cond.bind(cx), cx.nil())?;
                cx.lcons(h.bind(cx), a)
            })?;
            eval_keep_vis(cx, hcall, g)?;
        }
        cx.set_handler_stack(oldstack);
        let h = get_option1(cx, s, b"interrupt")?;
        if h.bind(cx).sexptype() != NILSXP {
            let o = s.out(cx);
            let call = o.root();
            cx.build(o, |cx| cx.lcons(h.bind(cx), cx.nil()))?;
            eval_keep_vis(cx, call, g)?;
        }
        Ok(())
    })
}

/// `getInterruptCondition()`: an empty list of class `c("interrupt",
/// "condition")`.
// R: errors.c:getInterruptCondition
fn interrupt_condition<'s>(cx: &mut Cx, s: Scope<'s>) -> RResult<Root<'s>> {
    let o = s.out(cx);
    let r = o.root();
    cx.build(o, |cx| {
        let mut k = cx.alloc_vector::<Char<'_>>(2)?;
        k.set(0, cx.mk_char(b"interrupt", Encoding::Native)?);
        k.set(1, cx.mk_char(b"condition", Encoding::Native)?);
        let k = k.finish();
        let mut l = cx.alloc_vector::<Ref<'_>>(0)?;
        let attrs = cx.cons_tagged(k.as_ref(), cx.nil(), cx.sym(RSym::Class))?;
        l.set_attrib(attrs);
        l.set_object(true);
        Ok(l.finish().into())
    })?;
    Ok(r)
}
