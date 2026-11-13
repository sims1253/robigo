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
    /// `R_RestartToken`: the handler of an internal restart's entry.
    pub restart_token: Global,
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
    /// `R_ShowErrorMessages` (`options(show.error.messages)`).
    pub show_error_messages: Cell<bool>,
    /// `R_ShowErrorCalls` (Rscript's profile sets `showErrorCalls = TRUE`
    /// in a non-interactive session), `R_ShowWarnCalls`, `R_NShowCalls`.
    pub show_error_calls: Cell<bool>,
    pub show_warn_calls: Cell<bool>,
    pub n_show_calls: Cell<usize>,
    /// `R_WarnLength` (`options(warning.length)`).
    pub warn_length: Cell<usize>,
}

impl CondState {
    /// The state of a new session.
    pub fn new(cx: &mut Cx) -> CondState {
        // R: names.c:InitNames (R_RestartToken = mkSymMarker(mkChar(""))):
        // any object no R code can make serves as the marker
        let restart = cx.alloc_vector_raw(VECSXP, 0).expect("robigo: no room for the condition system");
        // SAFETY: fresh, of this heap.
        let restart_token = cx.global(unsafe { Ref::from_sexp(restart, cx) });
        // R: errors.c:do_addCondHands, `allocVector(VECSXP, 1)`
        let token = cx.alloc_vector_raw(VECSXP, 1).expect("robigo: no room for the condition system");
        // SAFETY: as above.
        let handler_result_token = cx.global(unsafe { Ref::from_sexp(token, cx) });
        CondState {
            s3: crate::dispatch::S3Syms::new(cx),
            restart_token,
            handler_result_token,
            errbuf: RefCell::new(Vec::new()),
            in_error: Cell::new(0),
            in_warning: Cell::new(false),
            in_print_warnings: Cell::new(false),
            immediate_warning: Cell::new(false),
            no_break_warning: Cell::new(false),
            show_error_messages: Cell::new(true),
            show_error_calls: Cell::new(true),
            show_warn_calls: Cell::new(false),
            n_show_calls: Cell::new(50),
            warn_length: Cell::new(1000),
        }
    }
}

/// Install the condition system's hooks (the error, warning, exit-value
/// and dispatch entries of the isolate's hook table). Part of
/// [`crate::install`], after the state is in place.
pub(crate) fn install_hooks(cx: &mut Cx) {
    let mut h = cx.hooks();
    h.error = error_hook;
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
    cx.build(o, |cx| {
        let mut v = cx.alloc_vector::<Char<'_>>(1)?;
        v.set(0, cx.mk_char(b, Encoding::Native)?);
        Ok(v.finish().into())
    })?;
    Ok(r)
}

/// `GetOption1(install(name))` ([`crate::options::get_option1`]).
fn get_option1<'s>(cx: &mut Cx, s: Scope<'s>, name: &[u8]) -> RResult<Root<'s>> {
    crate::options::get_option1(cx, s, name)
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
pub fn error_hook(cx: &mut Cx, call: Option<Root<'_>>, msg: &str) -> RError {
    // a C string's message ends at a NUL
    let msg = msg.split('\0').next().unwrap_or_default().as_bytes();
    if let Some(e) = signal_error(cx, call, msg) {
        return e;
    }
    verrorcall_dflt(cx, call, msg)
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
                if h.sexp() == state(cx).restart_token.bind(cx).sexp() {
                    // go to default error handling
                    return Some(None);
                }
                // (R_OldCStackLimit: no C stack overflow is handled here)
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
        cx.find_context(ctxt::FUNCTION, rho, Some(result))
    })
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
    let nshow = state(cx).n_show_calls.get();
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
                    Some(s) => s.name().bytes().to_vec(),
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
            Some(s) => s.name().bytes().to_vec(),
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
    let warn_length = st.warn_length.get();
    // For use with Rv?snprintf, which truncates at size - 1, hence the + 1
    let msg_len = BUFSIZE.min(warn_length) + 1;
    let mut errbuf: Vec<u8> = Vec::new();
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
        if state(cx).show_error_calls.get()
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
    let show = state(cx).show_error_messages.get();
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
    // (R_OldCStackLimit: no C stack overflow is handled here)
    if try_user_handler && st.in_error.get() < 3 {
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
                        for i in 0..n {
                            let e = s.root(cx, opt.bind(cx).as_vector::<Ref<'_>>().unwrap().get(i));
                            let o = s.out(cx);
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
    // (only an interactive session, options(error) or catch.script.errors
    // save the traceback: R_Interactive is FALSE)
    if have_handler && traceback && state(cx).in_error.get() < 2 && state(cx).in_error.get() == old_in_error {
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
        Some((call, msg)) => cx.record_error_call(call, &String::from_utf8_lossy(msg)),
        None => {
            let top = cx.toplevel_context().id();
            cx.jump_to(top, 0, None)
        }
    }
}

/// `.Traceback`: the calls of the function and builtin records, each
/// duplicated (`R_GetTracebackOnly(0)`), as the symbol's value.
// R: errors.c:R_GetTracebackOnly
fn save_traceback(cx: &mut Cx) -> RResult<()> {
    cx.scope(|cx, s| {
        let calls: Vec<Root<'_>> = {
            let mut v = Vec::new();
            let mut c = Some(cx.global_context());
            while let Some(r) = c {
                if r.callflag() == ctxt::TOPLEVEL {
                    break;
                }
                if r.callflag() & (ctxt::FUNCTION | ctxt::BUILTIN) != 0 {
                    v.push(r.call());
                }
                c = r.next();
            }
            v.into_iter().map(|c| s.root(cx, c)).collect()
        };
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
        let sy = sym(cx, s, b".Traceback")?;
        let base = s.root(cx, cx.base_env());
        cx.define_var(sy, l, base)
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
                if exit.bind(cx).sexptype() == EXTPTRSXP {
                    // R_JumpToContext(c, CTXT_RESTART, R_RestartToken)
                    return cx.error("robigo: internal restarts (browser, try) are not implemented yet");
                }
                return cx.find_context(ctxt::FUNCTION, exit, Some(arglist));
            }
            cx.set_restart_stack(rest);
        }
        cx.error("restart not on stack")
    })
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
pub fn warning_hook(cx: &mut Cx, call: Option<Root<'_>>, msg: &str) -> RResult<()> {
    let msg = msg.split('\0').next().unwrap_or_default().as_bytes();
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
        let psize = BUFSIZE.min(st.warn_length.get() + 1);
        let buf = print_trunc(msg, psize);
        st.in_warning.set(true);
        let r = (|| -> RResult<()> {
            if w >= 2 {
                state(cx).in_warning.set(false);
                let mut m = b"(converted from warning) ".to_vec();
                m.extend_from_slice(&buf);
                return Err(cx.error_call(call, &String::from_utf8_lossy(&m)));
            } else if w == 1 {
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
                if state(cx).show_warn_calls.get()
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
                if state(cx).show_warn_calls.get()
                    && let Some(c) = call.map(|c| c.bind(cx)).filter(|c| c.sexptype() != NILSXP)
                {
                    let tr = concise_traceback(cx, Some(c), 0);
                    if !tr.is_empty() && tr.len() + buf.len() + 8 < BUFSIZE {
                        buf.extend_from_slice(b"\nCalls: ");
                        buf.extend_from_slice(&tr);
                    }
                }
                cx.collect_warning(call, &String::from_utf8_lossy(&buf));
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
        let item = |out: &mut Vec<u8>, call: Ref<'_>, msg: &[u8], indent: usize| {
            if call.sexptype() == NILSXP {
                out.extend_from_slice(msg);
                out.extend_from_slice(b" \n");
                return;
            }
            let dc = dcall(cx, call);
            out.extend_from_slice(b"In ");
            out.extend_from_slice(&dc);
            out.extend_from_slice(b" :");
            if indent + wd(&dc) + wd(line1(msg)) > LONGWARN {
                out.extend_from_slice(b"\n ");
            }
            out.push(b' ');
            out.extend_from_slice(msg);
            out.push(b'\n');
        };
        if n == 1 {
            out.extend_from_slice(b"Warning message:\n");
            let (c, m) = ws[0];
            item(&mut out, c, m.bytes(), 6);
        } else if n <= 10 {
            out.extend_from_slice(b"Warning messages:\n");
            for (i, &(c, m)) in ws.iter().enumerate() {
                out.extend_from_slice(format!("{}: ", i + 1).as_bytes());
                item(&mut out, c, m.bytes(), 10);
            }
        } else if n < robigo_rtcore::MAX_WARNINGS {
            out.extend_from_slice(format!("There were {n} warnings (use warnings() to see them)\n").as_bytes());
        } else {
            let m = robigo_rtcore::MAX_WARNINGS;
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
            let nsym = cx.install(b"names")?;
            let attrs = cx.cons_tagged(names.as_ref(), cx.nil(), nsym)?;
            l.set_attrib(attrs);
            Ok(l.finish().into())
        })?;
        let sy = sym(cx, s, b"last.warning")?;
        let base = s.root(cx, cx.base_env());
        cx.define_var(sy, v, base)
    })
}

// ---- the .Internals ---------------------------------------------------------------

/// `checkArity`: GNU R's message for an `.Internal` given the wrong number
/// of arguments.
fn check_arity(cx: &mut Cx, call: Root<'_>, args: Args<'_>, name: &str, n: usize) -> RResult<()> {
    if args.len() == n {
        return Ok(());
    }
    let k = args.len();
    let msg = format!(
        "{k} argument{} passed to .Internal({name}) which requires {n}",
        if k == 1 { "" } else { "s" }
    );
    Err(cx.error_call(Some(call), &msg))
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
/// first element translated, None when it is not a valid string
/// (`isValidString`).
fn message_arg(cx: &mut Cx, x: Root<'_>) -> RResult<Option<Vec<u8>>> {
    let v = x.bind(cx);
    if v.sexptype() == STRSXP {
        let s = v.as_vector::<Char<'_>>().unwrap();
        if s.len() == 0 || s.get(0) == cx.na_string() {
            // (NA_STRING is a valid string; an empty vector is not)
            return Ok((s.len() > 0).then(|| b"NA".to_vec()));
        }
        return Ok(Some(robigo_rtbuiltins::coerce::text(s.get(0)).bytes().to_vec()));
    }
    match robigo_rtbuiltins::coerce::Strings::of(cx, v) {
        Ok(s) if !s.is_empty() => Ok(Some(s.get_or_na(0).bytes().to_vec())),
        Ok(_) => Ok(None),
        Err(e) => Err(cx.error(&e.message())),
    }
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
                Some(msg) => Err(cx.error_call(c, &String::from_utf8_lossy(&msg))),
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
                Ok(Some(msg)) => cx.warning_call(c, &String::from_utf8_lossy(&msg)),
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
    if state(cx).show_error_messages.get() && !cx.collected_warnings().is_empty() {
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
        loop {
            let Some(list) = find_condition_handler(cx, cond.bind(cx)) else { break };
            let list = s.root(cx, list);
            let entry = s.root(cx, list.bind(cx).as_cons().unwrap().car());
            pop_handlers_to(cx, list);
            if is_calling(entry.bind(cx)) {
                let h = entry_elt(entry.bind(cx), entry::HANDLER);
                if h.sexp() == state(cx).restart_token.bind(cx).sexp() {
                    let m = msg.bind(cx);
                    let text = match m.as_vector::<Char<'_>>().filter(|v| m.sexptype() == STRSXP && v.len() > 0) {
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
            let csym = cx.install(b"class")?;
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
    cx.build(out, |cx| Ok(cx.alloc_copy(&[robigo_rtcore::Lgl(orig as i32)])?.finish().into()))
}

// ---- embedders (A1) ---------------------------------------------------------------

/// `R_tryEval(e, env, &errorOccurred)`: `eval(e, env)` in a top-level
/// context, the error (reported as GNU R reports it) turned into `None`;
/// `R_tryEvalSilent` with `silent` (no report: `R_ShowErrorMessages`
/// off). `R_Visible` is kept.
// R: context.c:R_tryEval, R_tryEvalSilent
pub fn try_eval(cx: &mut Cx, e: Root<'_>, env: Root<'_>, silent: bool, out: Out<'_>) -> RResult<bool> {
    let st = state(cx);
    let oldshow = st.show_error_messages.get();
    if silent {
        st.show_error_messages.set(false);
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
    state(cx).show_error_messages.set(oldshow);
    r
}

// ---- gettext ----------------------------------------------------------------------

/// `determine_domain_gettext(domain, up = TRUE)`: whether a domain applies
/// (a namespace's "R-<pkg>" found from the caller, or a string given).
/// GNU R's message catalogs translate nothing under `LANGUAGE=en`, so only
/// whether there is a domain matters: then each string is rebuilt
/// (`mkChar` of its translation).
// R: errors.c:determine_domain_gettext
fn has_domain(cx: &mut Cx, domain: Root<'_>) -> RResult<bool> {
    let d = domain.bind(cx);
    match d.sexptype() {
        NILSXP => {}
        STRSXP => {
            let v = d.as_vector::<Char<'_>>().unwrap();
            return Ok(v.len() > 0 && !v.get(0).bytes().is_empty());
        }
        LGLSXP if d.len() == Some(1) && robigo_rtbuiltins::util::as_logical(cx, d) == i32::MIN => return Ok(false),
        _ => return Err(cx.error("invalid 'domain' value")),
    }
    let g = cx.global_context();
    let mut rho = cx.empty_env();
    if g.callflag() & ctxt::FUNCTION != 0 {
        // the call is `f(<symbol>, domain = domain, ...)`: the domain of
        // f's caller
        let call = g.call();
        let shaped = (|| {
            let c = call.as_cons()?;
            c.car().as_sym()?;
            let a1 = c.cdr().as_cons()?;
            if a1.tag().sexptype() != NILSXP {
                return None;
            }
            a1.car().as_sym()?;
            let a2 = a1.cdr().as_cons()?;
            let t = a2.tag().as_sym()?;
            let v = a2.car().as_sym()?;
            (t.name().bytes() == b"domain" && v.name().bytes() == b"domain").then_some(())
        })()
        .is_some();
        rho = g.sysparent();
        if shaped && let Some(p) = cx.parent_context(g, 1) {
            rho = p.sysparent();
        }
    }
    let empty = cx.empty_env();
    let mut cnt = 0;
    while rho != empty && rho.sexptype() == ENVSXP {
        if rho == cx.global_env() {
            break;
        }
        if rho == cx.base_namespace() {
            // R_NamespaceEnvSpec(base): "base"
            return Ok(true);
        }
        cnt += 1;
        if cnt > 111 {
            break;
        }
        let robigo_rtcore::View::Env(e) = rho.view() else { break };
        let next = e.enclos();
        if next == rho {
            break;
        }
        rho = next;
    }
    // (no namespace but base's exists in robigo-rt)
    Ok(false)
}

/// `.Internal(gettext(domain, string, trim))`: the strings as their
/// translations (themselves: no catalog translates under `LANGUAGE=en`),
/// rebuilt as native strings when a domain applies.
// R: errors.c:do_gettext
pub fn do_gettext(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let nargs = args.len();
    if !(2..=3).contains(&nargs) {
        return Err(cx.error_call(Some(call), "either 2 or 3 arguments are required"));
    }
    let string = args.get(cx, 1);
    let n = string.len().unwrap_or(0);
    if string.sexptype() == NILSXP || n == 0 {
        out.set(cx, string);
        return Ok(());
    }
    if string.sexptype() != STRSXP {
        return Err(cx.error("invalid 'string' value"));
    }
    if !has_domain(cx, args.root(0))? {
        out.set(cx, args.get(cx, 1));
        return Ok(());
    }
    if nargs == 3 {
        // asRbool(trim, call): checked even though nothing is trimmed away
        let t = args.root(2);
        robigo_rtbuiltins::util::as_bool2(cx, t, call)?;
    }
    cx.build(out, |cx| {
        let v = args.get(cx, 1).as_vector::<Char<'_>>().unwrap();
        let mut a = cx.alloc_vector::<Char<'_>>(v.len())?;
        for i in 0..v.len() {
            // translateChar, then mkChar of the (identical) translation
            let b = match robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(v.get(i)), cx.locale()) {
                Ok(b) => b.into_owned(),
                Err(_) => v.get(i).bytes().to_vec(),
            };
            a.set(i, if v.get(i) == cx.na_string() { v.get(i) } else { cx.mk_char(&b, Encoding::Native)? });
        }
        Ok(a.finish().into())
    })
}
