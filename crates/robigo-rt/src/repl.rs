//! Rscript's top level over robigo-rt: `R --no-echo --file=FILE` reads the
//! file as console input, one statement at a time (`R_ReplConsole`,
//! `Rf_ReplIteration`), evaluates each in the global environment, prints a
//! visible value, then the deferred warnings; an error is reported and ends
//! the session ("Execution halted", exit status 1: `check_session_exit`
//! with no `options(error)`). `Rscript -e EXPR` is the same loop over the
//! expressions, one per line.
//!
//! An error is reported where it is raised, by the condition system
//! ([`crate::conditions`]: handlers, then `verrorcall_dflt`'s report),
//! before any `on.exit` code runs; what reaches the top level is the jump
//! there, after which Rscript halts. So does any other jump to the top
//! level (`invokeRestart("abort")`).
//!
//! Output goes to the isolate's console ([`Cx::rprintf`], [`Cx::reprintf`]);
//! values are printed by `PrintValueEnv` ([`crate::print`]).

use robigo_heap::types::*;
use robigo_rtcore::{Cx, RError, RResult, Ref, Root};

/// `LONGWARN`: the width at which an error or warning moves its message to
/// the next line.
const LONGWARN: usize = 75;
/// `R_WarnLength`: `options(warning.length)`'s default.
const WARN_LENGTH: usize = 1000;

/// Run `src` as Rscript runs a script file: true when it ran to the end,
/// false when an error ended it (the error written already; the caller
/// writes "Execution halted" and exits with status 1).
// R: main.c:R_ReplConsole, Rf_ReplIteration; unix/sys-std.c:Rstd_ReadConsole
pub fn run_console(cx: &mut Cx, src: &[u8]) -> bool {
    // setup_Rmainloop: `.Last.value` is NULL before the first statement
    let ok = cx.scope(|cx, s| {
        let nil = s.root(cx, cx.nil());
        match set_last_value(cx, nil) {
            Ok(()) => true,
            Err(err) => {
                // (reported where it was raised)
                cx.resolve_exit(err, None);
                false
            }
        }
    });
    if !ok {
        return false;
    }
    let arena = robigo_syntax::Arena::new();
    let mut iob: Vec<u8> = Vec::new();
    let mut incomplete = false;
    // R_ReadConsole: each line, a newline added to a last line without one
    let mut lines: Vec<&[u8]> = src.split_inclusive(|&b| b == b'\n').collect();
    let last_nl;
    let tail = match lines.last() {
        Some(l) if !l.ends_with(b"\n") => {
            last_nl = [*l, b"\n"].concat();
            lines.pop();
            Some(&last_nl[..])
        }
        _ => None,
    };
    let all = lines.into_iter().chain(tail);
    for line in all {
        let mut bufp = 0;
        while bufp < line.len() {
            // the console buffer takes the line up to the first ';' or
            // newline (not looking inside strings or comments)
            while bufp < line.len() {
                let c = line[bufp];
                bufp += 1;
                iob.push(c);
                if c == b';' || c == b'\n' {
                    break;
                }
            }
            let text = robigo_syntax::lexable(&iob);
            let source = robigo_syntax::Source::new(&text, &iob);
            let status = robigo_syntax::parse_console(&arena, source);
            incomplete = false;
            let ok = match status {
                robigo_syntax::ConsoleStatus::Null => true,
                robigo_syntax::ConsoleStatus::Eof => return true,
                robigo_syntax::ConsoleStatus::Incomplete(warnings) => {
                    if !warnings.is_empty() {
                        return parse_warnings_unsupported(cx);
                    }
                    incomplete = true;
                    continue;
                }
                robigo_syntax::ConsoleStatus::Error(e, warnings) => {
                    if !warnings.is_empty() {
                        return parse_warnings_unsupported(cx);
                    }
                    // R: source.c:parseError, `error("%s", msg)` with no
                    // call at the top level
                    let msg = parse_error_message(source, &e);
                    let err = cx.error_call(None, &msg);
                    cx.resolve_exit(err, None);
                    false
                }
                robigo_syntax::ConsoleStatus::Ok(exprs, warnings) => {
                    if !warnings.is_empty() {
                        return parse_warnings_unsupported(cx);
                    }
                    eval_statement(cx, &exprs[0])
                }
            };
            drop(text);
            iob.clear();
            if !ok {
                return false;
            }
        }
    }
    if incomplete {
        // R_ReplConsole: `error(_("unexpected end of input"))` at the top
        let err = cx.error_call(None, "unexpected end of input");
        cx.resolve_exit(err, None);
        return false;
    }
    true
}

/// The lexer's warnings (signalled as R warnings by GNU R's parse) need
/// the condition system to be raised at the top level.
fn parse_warnings_unsupported(cx: &Cx) -> bool {
    cx.reprintf(b"Error: robigo: the parser's warnings at the top level are not implemented yet\n");
    false
}

/// A parse error as the console reports it (`parseError(R_NilValue, 0)`).
// R: source.c:parseError, getParseContext
fn parse_error_message(src: robigo_syntax::Source<'_>, e: &robigo_syntax::ParseError) -> String {
    use robigo_syntax::ErrorStyle;
    match e.style {
        ErrorStyle::Condition => match e.call {
            None => robigo_syntax::condition_message(src, robigo_syntax::CONSOLE_SRCNAME, e),
            Some(_) => "robigo: a parse error condition with a call is not implemented yet".into(),
        },
        ErrorStyle::RError => e.message.clone(),
        ErrorStyle::Grammar => {
            let lines = robigo_syntax::parse_context(src.raw(), e.end);
            match lines.len() {
                0 => e.message.clone(),
                1 => format!("{} in \"{}\"", e.message, lines[0]),
                _ => format!("{} in:\n\"{}\n{}\"", e.message, lines[lines.len() - 2], lines[lines.len() - 1]),
            }
        }
    }
}

/// `Rf_ReplIteration`'s PARSE_OK branch: evaluate in the global
/// environment at the top level, set `.Last.value`, print a visible value,
/// then the deferred warnings. False when an error ended the statement.
// R: main.c:Rf_ReplIteration
fn eval_statement(cx: &mut Cx, expr: &robigo_syntax::Expr<'_>) -> bool {
    cx.scope(|cx, s| {
        let e = s.out(cx);
        let er = e.root();
        if let Err(err) = crate::reader::lower_one(cx, expr, e) {
            // (reported where it was raised)
            cx.resolve_exit(err, None);
            return false;
        }
        let g = s.root(cx, cx.global_env());
        let base = s.root(cx, cx.base_env());
        let o = s.out(cx);
        let or = o.root();
        let done = cx
            .toplevel(g, base, |cx| {
                cx.set_visible(false);
                crate::eval(cx, er, g, o).and_then(|()| {
                    set_last_value(cx, or)?;
                    if cx.visible() {
                        crate::print::print_value_env(cx, or, g)?;
                    }
                    Ok(())
                })
            })
            .expect("robigo: an exit that passed the top level");
        if !done {
            // a jump to the top level: the error (or restart) reported
            // where it was raised; Rscript halts (check_session_exit)
            return false;
        }
        print_warnings(cx);
        true
    })
}

/// `R_dot_Last()`, which `R_CleanUp` runs when a session ends normally
/// (Rscript's `--file` implies no save, so `runLast` holds): `.Last()` if
/// `.Last`, looked up from the global environment, is a closure; then
/// `.Last.sys()` from the base namespace the same way (only `R CMD
/// BATCH`'s profile defines one), each at the top level. False when an
/// error ended one: reported where it was raised, and Rscript halts
/// (`check_session_exit`).
///
/// As GNU R 4.6.2 does (`tests/rt/toplevel/dot-last-*.R`): neither call has
/// a context of its own, so an error in `.Last` jumps to the top level and
/// `check_session_exit`'s `R_CleanUp(SA_NOSAVE, 1, 0)` runs no
/// `.Last.sys`; the values go to `R_CurrentExpr`, not `.Last.value`.
// R: main.c:R_dot_Last
pub fn dot_last(cx: &mut Cx) -> bool {
    cx.scope(|cx, s| {
        let global = s.root(cx, cx.global_env());
        let base = s.root(cx, cx.base_env());
        let ns = s.root(cx, cx.base_namespace());
        call_if_closure(cx, b".Last", global, global) && call_if_closure(cx, b".Last.sys", ns, base)
    })
}

/// `R_dot_Last`'s step: `name()` evaluated in `rho` at the top level when
/// `findVar(name, from)` is a closure. The call's head is the symbol
/// (`lang1(cmd)`), so the call finds the function from `rho` again: a
/// `.Last.sys` in base (whose frame the base namespace shares) is found
/// from `R_BaseEnv`, one only in the global environment is found by
/// `findVar` through the namespace's parent but not by the call ("could
/// not find function"), as in GNU R.
fn call_if_closure(cx: &mut Cx, name: &[u8], from: Root<'_>, rho: Root<'_>) -> bool {
    cx.scope(|cx, s| {
        let sym = s.out(cx);
        let symr = sym.root();
        let f = s.out(cx);
        let fr = f.root();
        let call = s.out(cx);
        let callr = call.root();
        let looked_up = cx
            .build(sym, |cx| Ok(cx.install(name)?.as_ref()))
            .and_then(|()| cx.find_var(symr, from, f))
            .and_then(|()| {
                if fr.bind(cx).sexptype() != CLOSXP {
                    return Ok(false);
                }
                // lang1(cmd)
                cx.build(call, |cx| {
                    let nil = cx.nil();
                    cx.lcons(symr.bind(cx), nil)
                })?;
                Ok(true)
            });
        match looked_up {
            Ok(false) => return true,
            Ok(true) => {}
            Err(err) => {
                cx.resolve_exit(err, None);
                return false;
            }
        }
        let g = s.root(cx, cx.global_env());
        let base = s.root(cx, cx.base_env());
        let o = s.out(cx);
        cx.toplevel(g, base, |cx| crate::eval(cx, callr, rho, o)).expect("robigo: an exit that passed the top level")
    })
}

/// `SET_SYMVALUE(R_LastvalueSymbol, value)`: `.Last.value` in base.
fn set_last_value(cx: &mut Cx, v: Root<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let sym = s.out(cx);
        let symr = sym.root();
        cx.build(sym, |cx| Ok(cx.install(b".Last.value")?.as_ref()))?;
        let base = s.root(cx, cx.base_env());
        cx.define_var(symr, v, base)
    })
}

/// The text `verrorcall_dflt` writes for the pending error `e` (its message
/// and call), without the traceback: for errors outside a session (the
/// loader's), which no handler sees.
// R: errors.c:verrorcall_dflt
pub fn error_text(cx: &Cx, e: &RError) -> String {
    let (v, _, call) = cx.exit_objects(e);
    let msg = match v.as_scalar::<robigo_rtcore::Char<'_>>() {
        Some(c) => String::from_utf8_lossy(c.bytes()).into_owned(),
        None => format!("robigo: an exit of kind {:?} reached the top level", cx.exit_info(e)),
    };
    let mut buf = if call.sexptype() != NILSXP {
        let head = "Error in ";
        // Rvsnprintf_mbcs(tmp, msg_len - strlen(head)): at most this many
        // bytes, cut at a character boundary
        let msg = truncate_mbcs(&msg, (WARN_LENGTH + 1).saturating_sub(head.len()).saturating_sub(1));
        let dcall = deparse_call_line(cx, call);
        let mut b = format!("Error in {dcall} : ");
        let line1 = msg.split('\n').next().unwrap_or("");
        if 14 + wd(&dcall) + wd(line1) > LONGWARN {
            b.push_str("\n  ");
        }
        b.push_str(msg);
        b
    } else {
        let mut b = String::from("Error: ");
        b.push_str(truncate_mbcs(&msg, WARN_LENGTH.saturating_sub(b.len())));
        b
    };
    if !buf.ends_with('\n') {
        buf.push('\n');
    }
    buf
}

/// `PrintWarnings()`: the deferred warnings, then the list cleared
/// ([`crate::conditions::print_warnings`]).
pub fn print_warnings(cx: &mut Cx) {
    crate::conditions::print_warnings(cx);
}

/// `wd()`: the display width of UTF-8 text (`Ri18n_wcswidth`).
fn wd(s: &str) -> usize {
    s.chars().map(|c| robigo_text::ri18n_wcwidth(c as u32).max(0) as usize).sum()
}

/// At most `n` bytes of `s`, cut at a character boundary.
fn truncate_mbcs(s: &str, n: usize) -> &str {
    if s.len() <= n {
        return s;
    }
    let mut k = n;
    while !s.is_char_boundary(k) {
        k -= 1;
    }
    &s[..k]
}

/// `deparse1s(call)`: the first line of the call as errors and warnings
/// show it.
// R: deparse.c:deparse1s
pub fn deparse_call_line(cx: &Cx, call: Ref<'_>) -> String {
    match crate::deparse::deparse1s(cx, call) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(m) => format!("<{m}>"),
    }
}
