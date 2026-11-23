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
            // known_to_be_utf8: the session's (mkString2's UTF-8 mark)
            let source = robigo_syntax::Source::new(&text, &iob).known_to_be_utf8(cx.locale().utf8);
            let status = robigo_syntax::parse_console(&arena, source);
            incomplete = false;
            // PARSE_NULL, PARSE_OK and PARSE_ERROR start a new statement
            prompt_type = 1;
            let ok = match status {
                robigo_syntax::ConsoleStatus::Null => {
                    // (white space GNU R's C-locale lexer does not skip)
                    if crate::reader::non_ascii_refused(cx, &iob) {
                        error_at_top(cx, crate::reader::NON_ASCII_REFUSED);
                        false
                    } else {
                        true
                    }
                }
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
                    // (a non-ASCII statement GNU R's C-locale lexer refuses: a
                    // parse error, after the warnings of what it lexed)
                    if crate::reader::non_ascii_refused(cx, &iob) {
                        error_at_top_after(cx, &warnings, crate::reader::NON_ASCII_REFUSED);
                        false
                    } else {
                        eval_statement(cx, &exprs[0], &warnings, savestack)
                    }
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
pub(crate) fn error_at_top(cx: &mut Cx, msg: &str) {
    error_at_top_after(cx, &[], msg)
}

/// [`error_at_top`] after the lexer's `warnings`, raised at the top level
/// first ([`parse_warnings`]), so that the error report prints them.
pub(crate) fn error_at_top_after(cx: &mut Cx, warnings: &[String], msg: &str) {
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
pub(crate) fn check_session_exit(cx: &mut Cx) -> bool {
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
pub(crate) fn parse_error_message(src: robigo_syntax::Source<'_>, e: &robigo_syntax::ParseError) -> String {
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
pub(crate) fn call_if_closure(cx: &mut Cx, name: &[u8], from: Root<'_>, rho: Root<'_>) -> bool {
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

/// `.Internal(quit(save, status, runLast))`: `R_CleanUp(save, status,
/// runLast)`, then the process ends, from inside the call (GNU R's `exit`:
/// no `on.exit` or `finally` runs, the console is flushed; robigo-rt's file
/// connections write through, so nothing else is left in a buffer).
///
/// The save action: `"default"` and non-interactive `"ask"` are the command
/// line's `SaveAction`, which in robigo-rt is always `SA_NOSAVE` (it refuses
/// `--save`, and R without a console needs `--no-save` or `--vanilla`, or
/// implies them); `"yes"` saves the image as `R_SaveGlobalEnv` does, after
/// `.Last`: `sys.save.image(".RData")` (the image is dirty by then in GNU
/// R: the start removed `.Library.site` from the global environment).
/// With `runLast`, the context is cut back to the session's base record
/// first (`R_dot_Last`'s `R_GlobalContext = &R_Toplevel`): `.Last`,
/// `.Last.sys` and the save then see no frame of quit()'s callers
/// (`sys.calls()`), and an exiting handler around quit() finds "no function
/// to return from"; an error lands on the top level, where
/// `check_session_exit` halts R, or GNU R goes back to the main loop (and,
/// `.Last` failing again at the end, never ends), which robigo-rt refuses
/// (and halts). Without it the save runs in quit()'s context, its error
/// quit()'s. No browser context can be active (robigo
/// has no `browser`), so "cannot quit from browser" cannot arise; there is
/// no session temporary directory to remove and no device to close.
// R: main.c:do_quit, R_dot_Last; unix/sys-std.c:Rstd_CleanUp;
// saveload.c:R_SaveGlobalEnvToFile
pub fn do_quit(
    cx: &mut Cx,
    call: Root<'_>,
    args: robigo_rtcore::Args<'_>,
    _env: Root<'_>,
    _out: robigo_rtcore::Out<'_>,
) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "quit", 3, true)?;
    let save = args.get(cx, 0);
    if save.sexptype() != STRSXP {
        return Err(cx.error("one of \"yes\", \"no\", \"ask\" or \"default\" expected."));
    }
    let Some(tmp) =
        save.as_vector::<robigo_rtcore::Char<'_>>().filter(|v| !v.is_empty()).map(|v| v.get(0).bytes().to_vec())
    else {
        // STRING_ELT's bounds check
        return Err(cx.error("attempt access index 0/0 in STRING_ELT"));
    };
    let save = match &tmp[..] {
        b"ask" => {
            // (R_Interactive is FALSE)
            cx.warning("save=\"ask\" in non-interactive use: command-line default will be used")?;
            SaveAction::Ask
        }
        b"no" => SaveAction::NoSave,
        b"yes" => SaveAction::Save,
        b"default" => SaveAction::Default,
        _ => return Err(cx.error("unrecognized value of 'save'")),
    };
    let mut status = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(1))?;
    if status == i32::MIN {
        cx.warning("invalid 'status', 0 assumed")?;
        status = 0;
    }
    let mut run_last = robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 2));
    if run_last == i32::MIN {
        cx.warning("invalid 'runLast', FALSE assumed")?;
        run_last = 0;
    }
    match clean_up(cx, save, status, run_last != 0)? {}
}

/// `SA_TYPE` (Startup.h): what R does with the workspace as it ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveAction {
    /// The command line's (`SaveAction`).
    Default,
    NoSave,
    Save,
    /// Ask on the console (the command line's without one).
    Ask,
    /// R_Suicide's: nothing, no warnings.
    Suicide,
}

/// The command line's `SaveAction`, which `SA_DEFAULT` means at the end:
/// set by the front end (`--save`, `--no-save`, `--vanilla`, a FILE),
/// SA_NOSAVE until it does. A layer state of its own, not an `RtState`
/// field (a new field there moves the VM's reads).
struct SessionSave(std::cell::Cell<SaveAction>);

/// Set the command line's `SaveAction` (R_SetParams' `Rp->SaveAction`).
pub fn set_save_action(cx: &mut Cx, save: SaveAction) {
    match cx.ext::<SessionSave>() {
        Some(s) => s.0.set(save),
        None => cx.set_ext(SessionSave(std::cell::Cell::new(save))),
    }
}

fn session_save(cx: &Cx) -> SaveAction {
    cx.ext::<SessionSave>().map_or(SaveAction::NoSave, |s| s.0.get())
}

/// `R_CleanUp(save, status, runLast)` (Rstd_CleanUp): every way R ends goes
/// through it (quit(), the session's end, check_session_exit's halt,
/// R_Suicide). The save action resolved (`SA_DEFAULT`: the command line's;
/// `SA_SAVEASK` without a console: the command line's, the question being
/// the interactive console's, which robigo-rt does not have yet); then,
/// with `run_last`, R_dot_Last's cut of the context back to the top level,
/// `.Last` and `.Last.sys`, and the image saved there (SA_SAVE, when
/// `R_DirtyImage`: the global environment was assigned to or removed
/// from, which GNU R's start always does: it removes `.Library.site`);
/// without it the image
/// saved in the caller's context, so that an error in the save is the
/// caller's (quit()'s, catchable): the only way this returns. Then the
/// exit finalizers, (R_CleanTempDir: B's), the warnings unless SA_SUICIDE,
/// the console flushed, and the process ended with `status` (no unwinding:
/// GNU R's `exit`). An error in `.Last` or the save at the top level is
/// check_session_exit's: R halts (status 1), or GNU R goes back to the
/// main loop (and, `.Last` failing again at the end, never ends), which
/// robigo-rt refuses and halts.
// R: unix/sys-std.c:Rstd_CleanUp, main.c:R_dot_Last, saveload.c:R_SaveGlobalEnv
#[allow(unsafe_code)] // R_dot_Last's context reset (Cx::reset_to_session)
pub fn clean_up(cx: &mut Cx, save: SaveAction, status: i32, run_last: bool) -> RResult<std::convert::Infallible> {
    let mut sa = if save == SaveAction::Default { session_save(cx) } else { save };
    if sa == SaveAction::Ask {
        // (no console to ask on: the command line's; were that SA_SAVEASK
        // too, which R without a console does not allow, no save)
        sa = match session_save(cx) {
            SaveAction::Ask | SaveAction::Default => SaveAction::NoSave,
            s => s,
        };
    }
    let yes = sa == SaveAction::Save;
    if !run_last || sa == SaveAction::Suicide {
        if yes && cx.dirty_image() {
            save_global_env(cx)?;
        }
        exit(cx, status, sa == SaveAction::Suicide)
    }
    // R_dot_Last: the top level the context from here on. Nothing below may
    // come back into a frame the reset abandons, so it all runs under one
    // guard: its normal end is `exit`; an R error lands on a top-level
    // record pushed on the base (dot_last's, at_top's), and from there
    // check_session_exit's halt is `exit` too; a panic is caught at the
    // guard and ends the process with `abort`, before it could unwind
    // into the abandoned frames (do_quit's caller and up).
    // SAFETY: no record above the session's base is used again: every
    // way out of the guard below ends the process (exit or abort), and the
    // guard is entered right after the reset.
    unsafe { cx.reset_to_session() };
    let guarded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| after_reset(cx, yes, status)));
    match guarded {
        Ok(never) => match never {},
        Err(_) => {
            // (the panic's message was printed by the hook; this line's
            // write cannot panic, as eprintln! would on a stderr it cannot
            // write: nothing between here and abort may unwind)
            use std::io::Write;
            let _ = std::io::stderr().write_all(b"robigo: a panic after quit()'s context reset: aborting\n");
            std::process::abort()
        }
    }
}

/// `R_CleanUp` after R_dot_Last's context reset ([`clean_up`]): `.Last`,
/// `.Last.sys` and the save at the top level, then `exit`. Never returns;
/// its caller catches a panic.
fn after_reset(cx: &mut Cx, yes: bool, status: i32) -> std::convert::Infallible {
    // ROBIGO_PANIC_AFTER_RESET=1: a panic here, for the guard's test
    // (tests/boot_db.rs has none to make; read only on this path)
    if std::env::var_os("ROBIGO_PANIC_AFTER_RESET").is_some_and(|v| v == "1") {
        panic!("robigo: ROBIGO_PANIC_AFTER_RESET");
    }
    // (R_DirtyImage read after .Last, which may assign)
    let ok = dot_last(cx) && (!yes || !cx.dirty_image() || at_top(cx, save_global_env));
    if !ok {
        // the error landed on the top level: check_session_exit
        if check_session_exit(cx) {
            error_at_top(
                cx,
                "robigo: going back to the main loop after an error in quit()'s .Last or save is not implemented yet",
            );
        }
        // check_session_exit's R_CleanUp(SA_NOSAVE, 1, 0)
        cx.reprintf(b"Execution halted\n");
        exit(cx, 1, false);
    }
    exit(cx, status, false)
}

/// `f` at a REPL top-level record: false when an error ended it (reported
/// where it was raised).
fn at_top(cx: &mut Cx, f: fn(&mut Cx) -> RResult<()>) -> bool {
    cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        let base = s.root(cx, cx.base_env());
        cx.repl_toplevel(g, base, f).expect("robigo: an exit that passed the top level")
    })
}

/// `R_SaveGlobalEnvToFile(".RData")`: `sys.save.image(".RData")` evaluated
/// in the global environment when the global environment finds it (base
/// defines it); without it robigo-rt says so (GNU R's `R_SaveToFileV`).
// R: saveload.c:R_SaveGlobalEnvToFile
fn save_global_env(cx: &mut Cx) -> RResult<()> {
    cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        let f = s.out(cx);
        let fr = f.root();
        let call = s.out(cx);
        let callr = call.root();
        let o = s.out(cx);
        let sym = cx.install_rooted(s, b"sys.save.image")?;
        cx.find_var(sym, g, f)?;
        if fr.bind(cx).sexp() == cx.unbound().sexp() {
            return Err(cx.error(
                "robigo: saving the workspace without base's sys.save.image (R_SaveToFileV) is not implemented yet",
            ));
        }
        cx.build(call, |cx| {
            let name = cx.mk_string(b".RData", robigo_rtcore::Encoding::Native)?;
            let nil = cx.nil();
            let args = cx.lcons(name, nil)?;
            cx.lcons(sym.bind(cx), args)
        })?;
        crate::eval(cx, callr, g, o)
    })
}

/// The end of `Rstd_CleanUp` after its save action: the exit finalizers,
/// the deferred warnings (`.Last`'s, `quit`'s own), the console flushed,
/// then `exit(status)`.
// R: unix/sys-std.c:Rstd_CleanUp
fn exit(cx: &mut Cx, status: i32, suicide: bool) -> ! {
    clean_up_tail(cx, suicide);
    // (the isolate is not dropped: robigo's ROBIGO_GC_STATS report, which
    // the heap prints when it is, printed here, last, as it would be)
    if let Some(r) = cx.gc_stats_report() {
        eprint!("{r}");
    }
    std::process::exit(status)
}

/// What `Rstd_CleanUp` does after its save action ([`clean_up`]): the
/// exit finalizers, then (`R_CleanTempDir`: robigo-rt has no session
/// temporary directory yet, B's tempdir() port adds it here), then the
/// deferred warnings unless it is R_Suicide's (SA_SUICIDE), then the
/// console flushed. The caller ends the process with its status.
// R: unix/sys-std.c:Rstd_CleanUp
fn clean_up_tail(cx: &mut Cx, suicide: bool) {
    crate::fin::run_exit_finalizers(cx);
    if !suicide {
        print_warnings(cx);
    }
    cx.flush_console();
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
