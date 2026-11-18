//! R's top level over robigo-rt when it is not interactive: `R --file=FILE`
//! reads the file as console input, one line at a time, and runs it one
//! statement at a time (`R_ReplConsole`, `Rf_ReplIteration`): each is
//! evaluated in the global environment, a visible value printed, then the
//! deferred warnings; an error is reported and ends the session
//! ("Execution halted", exit status 1: `check_session_exit` with neither
//! `options(error)` nor `options(catch.script.errors)`). Rscript is `R --no-echo --file=FILE`, and `Rscript
//! -e EXPR` the same loop over the expressions, one per line; `R -f FILE`
//! (R CMD check's transcripts) also writes each line it reads after its
//! prompt, as a terminal would show it.
//!
//! An error is reported where it is raised, by the condition system
//! ([`crate::conditions`]: handlers, then `verrorcall_dflt`'s report),
//! before any `on.exit` code runs; what reaches the top level is the jump
//! there, after which `check_session_exit` halts Rscript or lets it read
//! on. So does any other jump to the top level (`invokeRestart("abort")`).
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
/// `CONSOLE_BUFFER_SIZE`: the console reader's buffer, of which `fgets`
/// fills at most all but one byte.
const CONSOLE_BUFFER_SIZE: usize = 4096;

/// Run `src` as R runs console input that is not interactive: true when it
/// ran to the end, false when an error ended it (the error written
/// already; the caller writes "Execution halted" and exits with status 1).
/// Unless `R_NoEcho` ([`Cx::no_echo`], checked at each read: `options(echo
/// = )` changes it), each line read is written after its prompt (`R -f
/// FILE`). The end of the input is the caller's (`end_Rmainloop`).
// R: main.c:R_ReplConsole, Rf_ReplIteration; unix/sys-std.c:Rstd_ReadConsole
pub fn run_console(cx: &mut Cx, src: &[u8]) -> bool {
    // setup_Rmainloop: `.Last.value` is NULL before the first statement
    let ok = cx.scope(|cx, s| {
        let nil = s.root(cx, cx.nil());
        match cx.set_last_value(nil) {
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
    // R_ReplConsole's `savestack`: each statement starts with the protect
    // stack's top back there
    let savestack = cx.pp_top();
    let arena = robigo_syntax::Arena::new();
    let mut iob: Vec<u8> = Vec::new();
    // R_ReplState: whether the last parse was PARSE_INCOMPLETE, and the
    // prompt the next read shows (1: a new statement, 2: inside one)
    let mut incomplete = false;
    let mut prompt_type = 1;
    let mut input = src;
    // Rf_ReplIteration: the next line is read once the last is used up
    loop {
        let line = match read_console(cx, &mut input, prompt_type) {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(err) => {
                // (reported where it was raised)
                cx.resolve_exit(err, None);
                return false;
            }
        };
        let mut bufp = 0;
        // each read is parsed at least once: an empty one (a NUL first) is
        // PARSE_EOF, or the statement it continues
        let mut fresh = true;
        while fresh || bufp < line.len() {
            fresh = false;
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
            // PARSE_NULL, PARSE_OK and PARSE_ERROR start a new statement
            prompt_type = 1;
            let ok = match status {
                robigo_syntax::ConsoleStatus::Null => true,
                // the parser may not have seen all of the input (PR#15941):
                // read on, as for an incomplete statement
                robigo_syntax::ConsoleStatus::Eof => {
                    prompt_type = 2;
                    continue;
                }
                robigo_syntax::ConsoleStatus::Incomplete(warnings) => {
                    // the trial parse's warnings, raised at the top level
                    // and printed before the next line is read
                    if !warnings.is_empty() {
                        if !parse_warnings(cx, &warnings) {
                            drop(text);
                            iob.clear();
                            if !check_session_exit(cx) {
                                return false;
                            }
                            incomplete = false;
                            prompt_type = 1;
                            break;
                        }
                        print_warnings(cx);
                    }
                    incomplete = true;
                    prompt_type = 2;
                    continue;
                }
                robigo_syntax::ConsoleStatus::Error(e, warnings) => {
                    // R: source.c:parseError, `error("%s", msg)` with no
                    // call at the top level, after the warnings of the
                    // tokens lexed up to the error ("In addition:")
                    let msg = parse_error_message(source, &e);
                    error_at_top_after(cx, &warnings, &msg);
                    false
                }
                robigo_syntax::ConsoleStatus::Ok(exprs, warnings) => {
                    eval_statement(cx, &exprs[0], &warnings, savestack)
                }
            };
            drop(text);
            iob.clear();
            if !ok {
                if !check_session_exit(cx) {
                    return false;
                }
                // run_Rmainloop's longjmp target: R_ReplConsole again, its
                // state reset, so the rest of this line is dropped and the
                // next read shows a new statement's prompt
                incomplete = false;
                prompt_type = 1;
                break;
            }
        }
    }
    if incomplete {
        // R_ReplConsole: `error(_("unexpected end of input"))` at the top;
        // past check_session_exit, R_ReplConsole again reads the end
        error_at_top(cx, "unexpected end of input");
        return check_session_exit(cx) && console_at_end(cx);
    }
    true
}

/// Rscript's session: the console's statements (`run_Rmainloop`), then
/// `end_Rmainloop`: its newline unless `R_NoEcho`, then `R_CleanUp(SA_DEFAULT,
/// 0, 1)`, which runs `.Last`. True when it ended normally; false when an
/// error halted it (the caller writes "Execution halted", runs no `.Last`
/// and exits with status 1).
///
/// An error in `.Last` jumps to `R_Toplevel`'s `SETJMP` in
/// `run_Rmainloop`, so `check_session_exit` decides there too: when the
/// session goes on, `R_ReplConsole` reads the end of the input at once and
/// `end_Rmainloop` runs again. As in GNU R, a `.Last` that always fails
/// then never ends.
// R: main.c:run_Rmainloop, end_Rmainloop, R_CleanUp (unix/sys-std.c)
pub fn run_session(cx: &mut Cx, src: &[u8]) -> bool {
    if !run_console(cx, src) {
        return false;
    }
    loop {
        // end_Rmainloop: `if (!R_NoEcho) Rprintf("\n")`, before .Last
        if !cx.no_echo() {
            cx.rprintf(b"\n");
        }
        if dot_last(cx) {
            return true;
        }
        if !check_session_exit(cx) || !console_at_end(cx) {
            return false;
        }
    }
}

/// `R_ReplConsole` once its input is used up, as after `check_session_exit`
/// let the session go on at the end: its read writes the prompt (unless
/// `R_NoEcho`) and finds the end. False if writing the prompt failed (the
/// error reported).
// R: main.c:R_ReplConsole, unix/sys-std.c:Rstd_ReadConsole
fn console_at_end(cx: &mut Cx) -> bool {
    match read_console(cx, &mut &b""[..], 1) {
        Ok(_) => true,
        Err(err) => {
            // (reported where it was raised)
            cx.resolve_exit(err, None);
            false
        }
    }
}

/// An error raised at the top level, as the console's parse error and its
/// "unexpected end of input" are (`error()` with `R_Toplevel` the context):
/// reported where it is raised, by the handlers then the default handling.
/// Its jump lands on a REPL top-level record, so `R_HandlerStack` and
/// `R_RestartStack`, which the error's signal left as it found them or
/// emptied, are restored from the top level's saved stacks, the global
/// calling handlers among them.
// R: errors.c:jump_to_top_ex, context.c:R_restore_globals
fn error_at_top(cx: &mut Cx, msg: &str) {
    error_at_top_after(cx, &[], msg)
}

/// [`error_at_top`] after the lexer's `warnings`, raised at the top level
/// first ([`parse_warnings`]), so that the error report prints them.
fn error_at_top_after(cx: &mut Cx, warnings: &[String], msg: &str) {
    cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        let base = s.root(cx, cx.base_env());
        let landed = cx
            .repl_toplevel(g, base, |cx| {
                warnings.iter().try_for_each(|w| cx.warning(w))?;
                Err(cx.error_call(None, msg))
            })
            .expect("robigo: an exit that passed the top level");
        debug_assert!(!landed, "robigo: an error that returned");
    })
}

/// `check_session_exit()`, run after a jump to the top level: a script
/// goes on (true) when `options(error)` is set or
/// `options(catch.script.errors)` is TRUE; otherwise it halts (false: the
/// caller writes "Execution halted" and quits with status 1, running no
/// `.Last`). R_Interactive is FALSE.
// R: main.c:check_session_exit
fn check_session_exit(cx: &mut Cx) -> bool {
    cx.scope(|cx, s| {
        let go_on = (|| {
            let error = robigo_rtbuiltins::options::get_option(cx, s, b"error")?;
            if error.bind(cx).sexptype() != NILSXP {
                return Ok(true);
            }
            // R_isTRUE
            let c = robigo_rtbuiltins::options::get_option(cx, s, b"catch.script.errors")?;
            let c = c.bind(cx);
            Ok::<bool, RError>(c.sexptype() == LGLSXP && c.as_ints() == Some(&[1][..]))
        })();
        match go_on {
            Ok(b) => b,
            Err(err) => {
                cx.resolve_exit(err, None);
                false
            }
        }
    })
}

/// `R_ReadConsole(R_PromptString(0, prompt_type), buf, CONSOLE_BUFFER_SIZE,
/// 1)` when R is not interactive: the prompt written unless `R_NoEcho`, then
/// the next line of `input` as `fgets` reads it (through its newline, at
/// most `CONSOLE_BUFFER_SIZE - 1` bytes, `strlen`'s up to a NUL), its CRLF
/// as LF, a newline added to the input's last line, the line written unless
/// `R_NoEcho`. None at the end of the input, after the prompt all the same.
///
/// GNU R writes both with `fputs` to the process's `stdout`, past any
/// `sink()`; here that is the console's output, which robigo-rt has no
/// `sink()` for yet.
// R: unix/sys-std.c:Rstd_ReadConsole
fn read_console(cx: &mut Cx, input: &mut &[u8], prompt_type: i32) -> RResult<Option<Vec<u8>>> {
    let echo = !cx.no_echo();
    if echo {
        let prompt = prompt_string(cx, prompt_type)?;
        cx.rprintf(&prompt);
    }
    if input.is_empty() {
        return Ok(None);
    }
    let len = CONSOLE_BUFFER_SIZE;
    let n = match input.iter().take(len - 1).position(|&b| b == b'\n') {
        Some(i) => i + 1,
        None => input.len().min(len - 1),
    };
    let (read, rest) = input.split_at(n);
    *input = rest;
    // feof: fgets stopped at the end of the input, not at a newline or the
    // end of the buffer
    let eof = !read.ends_with(b"\n") && read.len() < len - 1;
    let ll = read.iter().position(|&b| b == 0).unwrap_or(read.len());
    let mut buf = read[..ll].to_vec();
    // remove CR in CRLF ending
    if buf.ends_with(b"\r\n") {
        buf.truncate(buf.len() - 2);
        buf.push(b'\n');
    }
    // according to system.txt, should be terminated in \n
    if eof && !buf.ends_with(b"\n") && buf.len() < len {
        buf.push(b'\n');
    }
    if echo {
        cx.rprintf(&buf);
    }
    Ok(Some(buf))
}

/// `R_PromptString(0, prompt_type)` with echo: `getOption("prompt")` for a
/// new statement, `getOption("continue")` inside one (the browser's prompt
/// is `browser()`'s console, which this loop is not).
// R: main.c:R_PromptString
fn prompt_string(cx: &mut Cx, prompt_type: i32) -> RResult<Vec<u8>> {
    let name: &[u8] = if prompt_type == 1 { b"prompt" } else { b"continue" };
    cx.scope(|cx, s| {
        let v = robigo_rtbuiltins::options::get_option(cx, s, name)?;
        // CHAR(STRING_ELT(v, 0)): options() keeps both a string
        let first = v
            .bind(cx)
            .as_vector::<robigo_rtcore::Char<'_>>()
            .filter(|v| !v.is_empty())
            .map(|v| v.get(0).bytes().to_vec());
        match first {
            Some(b) => Ok(b),
            None => Err(cx.error(&format!("robigo: getOption(\"{}\") is not a string", String::from_utf8_lossy(name)))),
        }
    })
}

/// The lexer's warnings, raised as the parser raises them, `warning()` at
/// the top level (R_Toplevel: no call); collected for `PrintWarnings`,
/// or printed or made an error as `options(warn)` says. False when one
/// became an error (reported at the top).
// R: gram.y:NumericValue, mkString2 (warning), errors.c:warning
fn parse_warnings(cx: &mut Cx, warnings: &[String]) -> bool {
    cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        let base = s.root(cx, cx.base_env());
        cx.repl_toplevel(g, base, |cx| warnings.iter().try_for_each(|w| cx.warning(w)))
            .expect("robigo: an exit that passed the top level")
    })
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
fn eval_statement(cx: &mut Cx, expr: &robigo_syntax::Expr<'_>, warnings: &[String], savestack: i32) -> bool {
    // under PROTECT(thisExpr), then PROTECT(value) for the print
    crate::base::repl_eval_toplevel(cx, expr, warnings, |cx| cx.global_env(), savestack, (1, 2)).is_ok()
}

/// `R_dot_Last()`, which `R_CleanUp` runs when a session ends normally
/// (Rscript's `--file` implies no save, so `runLast` holds): `.Last()` if
/// `.Last`, looked up from the global environment, is a closure; then
/// `.Last.sys()` from the base namespace the same way (only `R CMD
/// BATCH`'s profile defines one), each at the top level. False when an
/// error ended one: reported where it was raised, and
/// [`run_session`] asks `check_session_exit` whether the session halts.
///
/// As GNU R 4.6.2 does (`tests/rt/toplevel/dot-last-*.R`): neither call has
/// a context of its own, so an error in `.Last` jumps to the top level and
/// `check_session_exit`'s `R_CleanUp(SA_NOSAVE, 1, 0)` runs no
/// `.Last.sys`; the values go to `R_CurrentExpr`, not `.Last.value`.
// R: main.c:R_dot_Last
fn dot_last(cx: &mut Cx) -> bool {
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
///
/// The lookup and the call both run in R_Toplevel (`R_dot_Last` sets it
/// the global context), so an error in either lands on a REPL top-level
/// record.
fn call_if_closure(cx: &mut Cx, name: &[u8], from: Root<'_>, rho: Root<'_>) -> bool {
    cx.scope(|cx, s| {
        let sym = s.out(cx);
        let symr = sym.root();
        let f = s.out(cx);
        let fr = f.root();
        let call = s.out(cx);
        let callr = call.root();
        let o = s.out(cx);
        let g = s.root(cx, cx.global_env());
        let base = s.root(cx, cx.base_env());
        cx.repl_toplevel(g, base, |cx| {
            cx.build(sym, |cx| Ok(cx.install(name)?.as_ref()))?;
            cx.find_var(symr, from, f)?;
            if fr.bind(cx).sexptype() != CLOSXP {
                return Ok(());
            }
            // lang1(cmd)
            cx.build(call, |cx| {
                let nil = cx.nil();
                cx.lcons(symr.bind(cx), nil)
            })?;
            crate::eval(cx, callr, rho, o)
        })
        .expect("robigo: an exit that passed the top level")
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
