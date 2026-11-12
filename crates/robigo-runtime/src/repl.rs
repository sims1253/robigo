//! R's non-interactive console: how `R -f FILE` reads a file as console
//! input and prints the transcript that `R CMD check` compares. `Rscript
//! FILE` is the same loop with prompt and echo suppressed (`--no-echo`),
//! and `R -e EXPR` feeds the expressions through it as a temporary file.
//!
//! Ports:
//! - The console stream itself (`R: unix/sys-std.c:Rstd_ReadConsole`:
//!   prompt, echo, line reads) is `crate::console`, which the
//!   `stdin()` connection reads too.
//! - `R: main/main.c:Rf_ReplIteration` and `R_ReplConsole`: how much
//!   input each parse sees (up to the first `;` or newline, wherever it
//!   lies), the parse statuses, the prompts (`>` and `+`), and the
//!   `unexpected end of input` when the input ends mid-statement.
//! - `R: main/main.c:check_session_exit`: an error ends the session
//!   unless `options(error =)` or `options(catch.script.errors = TRUE)`
//!   says otherwise.
//! - `R: main/main.c:end_Rmainloop`: the final newline.
//! - `R: main/source.c:parseError` (with `linenum == 0`) and
//!   `source.c:getParseContext`: the context a parse error reports.

use crate::builtins::parsing::ParseFailure;
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::lang::expr_to_value_src;
use crate::value::{TRUE, Value};
use crate::{Interp, Sym};

/// The parse statuses of `R_Parse1Buffer` the REPL distinguishes (R:
/// main.c:Rf_ReplIteration's switch over `state->status`).
enum Status<'a> {
    Null,
    /// PARSE_EOF: `prog: END_OF_INPUT` before anything else, which a
    /// statement starting with an invalid character whose bytes run past
    /// the chunk produces (gram.y:mbcs_get_next): the REPL ends there.
    Eof,
    Ok(Vec<robigo_syntax::Expr<'a>>),
    Incomplete,
    Error(robigo_syntax::ParseError),
}

/// Like `R_PromptString` (R: main.c): the `prompt` or `continue` option
/// (re-read every line, so R code can change them), or nothing in
/// no-echo mode.
pub fn prompt_string(interp: &Interp, echo: bool, continue_line: bool) -> String {
    if !echo {
        return String::new();
    }
    let option = |name: &str, default: &str| -> String {
        match interp.get_option(name) {
            Some(Value::Str(v)) if !v.data.is_empty() => {
                v.data[0].as_deref().map(str::to_string).unwrap_or_else(|| default.to_string())
            }
            _ => default.to_string(),
        }
    };
    if continue_line { option("continue", "+ ") } else { option("prompt", "> ") }
}

/// One `R_ReadConsole` call (R: unix/sys-std.c:Rstd_ReadConsole): the
/// prompt is printed, a line is read from the console stream and echoed,
/// exactly as read. Returns `None` on end of input.
fn read_console(prompt: &str) -> Option<Vec<u8>> {
    crate::console::read_console(prompt)
}

/// `R_Parse1Buffer` with `gend = 0` (R: gram.c:R_Parse1 over the grammar
/// `prog: END_OF_INPUT | '\n' | expr '\n' | expr ';' | error`): does the
/// buffer hold a complete statement, is it incomplete, or is it an error?
/// The REPL feeds the buffer up to the first `;` or newline, so a
/// complete buffer is always one terminated statement. The lexer's
/// warnings are returned for the caller to signal, in the order GNU R's
/// trial and (for a complete statement) real parses raise them.
fn parse_status<'a>(arena: &'a robigo_syntax::Arena, buf: robigo_syntax::Source<'a>) -> (Status<'a>, Vec<String>) {
    match robigo_syntax::parse_console(arena, buf) {
        robigo_syntax::ConsoleStatus::Null => (Status::Null, Vec::new()),
        robigo_syntax::ConsoleStatus::Ok(exprs, warnings) => (Status::Ok(exprs), warnings),
        robigo_syntax::ConsoleStatus::Incomplete(warnings) => (Status::Incomplete, warnings),
        robigo_syntax::ConsoleStatus::Error(e, warnings) => (Status::Error(e), warnings),
        // The REPL's chunks are never empty (a read returns at least one
        // byte), so only an invalid character ends one before it starts.
        robigo_syntax::ConsoleStatus::Eof => (Status::Eof, Vec::new()),
    }
}

/// A parse error as the REPL reports it. Only grammar errors (`yyerror`)
/// reach `parseError(R_NilValue, 0)` (R: main/source.c), which appends the
/// last lines of the parse context; conditions raised by `raiseParseError`
/// and `raiseLexError` carry their own `(<input>:<line>:<col>)` suffix,
/// and a plain `error()` from a parser action (`names.c:install`) has no
/// position or context at all.
fn parse_error_message<'a>(buf: impl Into<robigo_syntax::Source<'a>>, e: &robigo_syntax::ParseError) -> String {
    use robigo_syntax::ErrorStyle;
    let buf = buf.into();
    match e.style {
        ErrorStyle::Condition => robigo_syntax::condition_message(buf, robigo_syntax::CONSOLE_SRCNAME, e),
        ErrorStyle::RError => e.message.clone(),
        ErrorStyle::Grammar => {
            let lines = robigo_syntax::parse_context(buf.raw(), e.end);
            match lines.len() {
                0 => e.message.clone(),
                1 => format!("{} in \"{}\"", e.message, lines[0]),
                _ => format!("{} in:\n\"{}\n{}\"", e.message, lines[lines.len() - 2], lines[lines.len() - 1]),
            }
        }
    }
}

/// The error the REPL raises for a parse error. A `parseError`
/// condition from `raiseParseError`/`raiseLexError` keeps its call
/// attribution even in the REPL (gram.y:R_Parse1 passes `R_CurrentExpr`
/// for the pipe checks), so `1 |> f(_)` reports `Error in f("_") : ...`;
/// the others are `errorcall(R_NilValue, msg)` (`Error: ...`).
fn parse_error_condition<'a>(buf: impl Into<robigo_syntax::Source<'a>>, e: &robigo_syntax::ParseError) -> ParseFailure {
    let buf = buf.into();
    match e.style {
        robigo_syntax::ErrorStyle::Condition => {
            crate::builtins::parsing::parse_condition(buf, robigo_syntax::CONSOLE_SRCNAME, e)
        }
        _ => ParseFailure::Error { call: Some(Value::Null), message: parse_error_message(buf, e) },
    }
}

/// Did the session set an error handler that keeps it alive after an
/// error (R: main.c:check_session_exit: a non-NULL `error` option or
/// `catch.script.errors = TRUE`)?
fn catches_script_errors(interp: &Interp) -> bool {
    interp.get_option("error").map(|o| !o.is_null()).unwrap_or(false)
        || crate::coerce::as_logical(&interp.get_option("catch.script.errors").unwrap_or(Value::Null)) == TRUE
}

/// `R_ReplState` (R: main.c): the console line being worked through and
/// the statement collected from it so far.
struct ReplState {
    line: Vec<u8>,           // state->buf
    bufp: usize,             // state->bufp
    iob: Vec<u8>,            // R_ConsoleIob
    prompt_type: u8,         // 1: `> `, 2: `+ `
    status_incomplete: bool, // state->status == PARSE_INCOMPLETE
}

impl ReplState {
    fn new() -> ReplState {
        ReplState { line: Vec::new(), bufp: 0, iob: Vec::new(), prompt_type: 1, status_incomplete: false }
    }

    /// What a longjmp to a restarted REPL leaves: the statement and the
    /// rest of the current line are dropped and reading continues at the
    /// next one.
    fn reset(&mut self) {
        *self = ReplState::new();
    }

    /// Rf_ReplIteration up to the parse: read a line when the last one is
    /// used up (prompting with `prompt`), then feed the console buffer up
    /// to the first `;` or newline (the copy loop does not look inside
    /// strings or comments). `false` at end of input.
    fn feed(&mut self, interp: &mut Interp, prompt: impl FnOnce(&Interp, u8) -> String) -> bool {
        if self.bufp >= self.line.len() {
            // R_ReadConsole: buffered output goes out before the prompt.
            interp.flush();
            let prompt = prompt(interp, self.prompt_type);
            match read_console(&prompt) {
                Some(l) => {
                    self.line = l;
                    self.bufp = 0;
                }
                None => return false,
            }
        }
        while self.bufp < self.line.len() {
            let c = self.line[self.bufp];
            self.bufp += 1;
            self.iob.push(c);
            if c == b';' || c == b'\n' {
                break;
            }
        }
        true
    }
}

/// `R_ReplConsole` over the console: read a line, echo it, parse and
/// evaluate one statement at a time until end of input or an error.
/// Returns `false` when the session must halt with "Execution halted"
/// (whatever failed has been written already).
// R: main.c:R_ReplConsole, Rf_ReplIteration, end_Rmainloop
pub fn repl_console(interp: &mut Interp, echo: bool) -> bool {
    let mut st = ReplState::new();
    let global = interp.global.clone();
    let arena = robigo_syntax::Arena::new();
    loop {
        if !st.feed(interp, |interp, prompt_type| prompt_string(interp, echo, prompt_type == 2)) {
            // R_ReplConsole: end of input while a statement is still open
            // is an error.
            if st.status_incomplete {
                if !interp.signal_parse_conditions(
                    Vec::new(),
                    Some(ParseFailure::Error { call: Some(Value::Null), message: "unexpected end of input".into() }),
                ) && !catches_script_errors(interp)
                {
                    return false;
                }
                // check_session_exit continued: the restarted REPL reads
                // the end of input again, with no statement open.
                st.reset();
                continue;
            }
            // end_Rmainloop: `Rprintf("\n")`, except in no-echo mode.
            if echo {
                interp.write_out("\n");
                interp.flush();
            }
            return true;
        }
        // The lexer reads `text`, the chunk with each invalid UTF-8 byte
        // replaced (robigo_syntax::Source); errors and source references
        // are reported against it.
        let text = robigo_syntax::lexable(&st.iob);
        let source = robigo_syntax::Source::new(&text, &st.iob);
        let (status, warns) = parse_status(&arena, source);
        st.status_incomplete = matches!(status, Status::Incomplete);
        // The statement is done with (R resets R_ConsoleIob); cleared once
        // `text`, which may borrow it, is gone.
        let mut done = false;
        let ok = match status {
            Status::Null => {
                done = true;
                st.prompt_type = 1;
                true
            }
            // R: main.c:Rf_ReplIteration -- `case PARSE_EOF: return(-1);`.
            // R_ReplConsole then raises "unexpected end of input" only if
            // `state.status == PARSE_INCOMPLETE`, and R_Parse1Buffer has
            // just stored PARSE_EOF there (as `status_incomplete` above
            // follows every parse), so the session ends without an error
            // whatever the chunk before was: end_Rmainloop.
            Status::Eof => {
                if echo {
                    interp.write_out("\n");
                    interp.flush();
                }
                return true;
            }
            Status::Incomplete => {
                st.prompt_type = 2;
                // Rf_ReplIteration prints pending warnings at the top of
                // each iteration: the parse's warnings are out before
                // the next line is read (e.g. an unpaired surrogate in a
                // statement that is still open).
                let ok = interp.signal_parse_conditions(warns, None);
                if ok {
                    interp.print_warnings();
                }
                ok
            }
            // The parse's warnings signal before the error; with
            // `options(warn = 2)` the first of them is the error.
            Status::Error(ref e) => interp.signal_parse_conditions(warns, Some(parse_error_condition(source, e))),
            Status::Ok(ref exprs) => {
                // The real parse (R's `gencode = 1`) signals the lexer's
                // warnings while building the expression; with
                // `options(warn = 2)` they are errors and stop the
                // statement before it evaluates.
                interp.signal_parse_conditions(warns, None) && {
                    // Rf_ReplIteration's PARSE_OK branch.
                    let v = expr_to_value_src(&exprs[0], Some(("<text>", &text)));
                    let value = interp.eval_toplevel(&v, &global, true);
                    done = true;
                    st.prompt_type = 1;
                    match value {
                        Some(val) => {
                            // SET_SYMVALUE(R_LastvalueSymbol, value): a
                            // binding in the base environment, not the
                            // global one
                            interp.base.clone().force_set(Sym::new(".Last.value"), val.clone());
                            // Rf_ReplIteration's PARSE_OK branch ends with
                            // Rf_callToplevelHandlers(lastExpr, value, TRUE,
                            // wasDisplayed); a warning converted to an
                            // error (options(warn = 2)) longjmps out of it
                            // to the top level, like an evaluation error.
                            let visible = interp.toplevel_visible;
                            let failed = if interp.task_callbacks.is_empty() {
                                None
                            } else {
                                interp.call_toplevel_handlers(&v, &val, visible)
                            };
                            match failed {
                                Some(f) => {
                                    interp.report_toplevel(f);
                                    false
                                }
                                None => true,
                            }
                        }
                        None => false,
                    }
                }
            }
        };
        drop(text);
        if done {
            st.iob.clear();
        }
        if !ok {
            // The error longjmped to the top level, whatever raised it.
            if !catches_script_errors(interp) {
                return false;
            }
            // check_session_exit continued the session: R's longjmp lands
            // in R_ReplConsole's caller, which restarts the REPL.
            st.reset();
        }
    }
}

/// A running browser (R's `CTXT_BROWSER` context), innermost last in
/// `Interp::browsers`.
pub struct Browser {
    /// The `text` and `condition` arguments, for `browserText()` and
    /// `browserCondition()` (R: the context's `promargs`).
    pub text: Value,
    pub condition: Value,
    /// How many contexts were on `Interp::frames` when it started:
    /// `browserSetDebug()` counts function contexts from there.
    pub frames: usize,
}

/// What one `Rf_ReplIteration` in a browser tells `R_ReplConsole`.
enum Step {
    /// Go on reading (R's 0, 1 and 2).
    More,
    /// Leave the browser (R's -1): `c`, `n`, `s`, an empty line, the end
    /// of input.
    Leave,
}

/// `R_PromptString(browselevel, type)` (R: main.c).
fn browser_prompt(interp: &Interp, level: usize, prompt_type: u8) -> String {
    if !crate::console::echo() {
        String::new()
    } else if prompt_type == 1 {
        format!("Browse[{level}]> ")
    } else {
        prompt_string(interp, true, true)
    }
}

/// `printwhere` (R: main.c): the calls on the stack, innermost first.
fn print_where(interp: &mut Interp) -> RResult<()> {
    let calls: Vec<Value> =
        interp.frames.iter().rev().map(|f| f.call.clone()).filter(|c| matches!(c, Value::Lang(_))).collect();
    for (i, call) in calls.iter().enumerate() {
        // SrcrefPrompt("", cptr->srcref): robigo's contexts keep no
        // srcref of their own.
        interp.write_out(&format!("where {}: ", i + 1));
        let global = interp.global.clone();
        interp.print_value_toplevel(call, &global)?;
    }
    interp.write_out("\n");
    Ok(())
}

/// `ParseBrowser` (R: main.c): the browser's commands. 0: evaluate the
/// expression, 1: leave the browser, 2: read the next one.
fn parse_browser(interp: &mut Interp, expr: &Value, rho: &Env) -> RResult<u8> {
    let Some(sym) = expr.as_sym() else { return Ok(0) };
    Ok(match &*sym.name() {
        "c" | "cont" => {
            rho.set_rdebug(false);
            1
        }
        "f" => {
            // R marks the nearest loop or function context to finish
            // (`browserfinish`) and keeps stepping after it.
            return Err(Flow::error("robigo: browser command 'f' is not implemented yet"));
        }
        "help" => {
            interp.write_out(
                "n          next\ns          step into\nf          finish\nc or cont  continue\nQ          quit\n\
                 where      show stack\nhelp       show help\n<expr>     evaluate expression\n",
            );
            2
        }
        "n" => {
            rho.set_rdebug(true);
            interp.browser_last_command = b'n';
            1
        }
        "Q" => {
            // this is really dynamic state that should be managed as such
            rho.set_rdebug(false);
            // jump_to_toplevel: no message, no handlers, past every
            // browser and try.
            interp.print_warnings();
            return Err(Flow::Error(Box::new(crate::error::RError {
                message: String::new(),
                call: None,
                signaled: true,
                classes: vec![],
                fields: vec![],
            })));
        }
        "s" => {
            rho.set_rdebug(true);
            interp.browser_last_command = b's';
            1
        }
        "where" => {
            print_where(interp)?;
            2
        }
        "r" => {
            let hook = Sym::new(".tryResumeInterrupt");
            if interp.base.has_local(hook) {
                let global = interp.global.clone();
                interp.eval(&Value::lang(Value::Sym(hook), vec![]), &global)?;
            }
            0
        }
        _ => 0,
    })
}

/// One `Rf_ReplIteration` at browser level `level`, evaluating in `rho`.
/// A failed evaluation comes back as its `Flow`.
// R: main.c:Rf_ReplIteration (browselevel > 0)
fn browser_iteration(
    interp: &mut Interp,
    rho: &Env,
    level: usize,
    st: &mut ReplState,
    arena: &robigo_syntax::Arena,
) -> RResult<Step> {
    if !st.feed(interp, |interp, prompt_type| browser_prompt(interp, level, prompt_type)) {
        // R_ReplConsole: end of input while a statement is still open
        // is an error.
        if st.status_incomplete {
            st.status_incomplete = false;
            return Err(Flow::error("unexpected end of input"));
        }
        return Ok(Step::Leave);
    }
    // The statement leaves the buffer (R resets R_ConsoleIob) unless it is
    // incomplete; a failure restarts the browser's REPL with a fresh one.
    let iob = std::mem::take(&mut st.iob);
    let text = robigo_syntax::lexable(&iob);
    let source = robigo_syntax::Source::new(&text, &iob);
    let (status, warns) = parse_status(arena, source);
    st.status_incomplete = matches!(status, Status::Incomplete);
    match status {
        Status::Null => {
            // The intention here is to break on CR but not on other null
            // statements: see PR#9063 (R_DisableNLinBrowser is FALSE).
            if st.line == b"\n" {
                return Ok(Step::Leave);
            }
            st.prompt_type = 1;
            Ok(Step::More)
        }
        // R: main.c:Rf_ReplIteration -- `case PARSE_EOF: return(-1);`.
        Status::Eof => Ok(Step::Leave),
        Status::Incomplete => {
            drop(text);
            st.iob = iob;
            st.prompt_type = 2;
            interp.signal_parse_conditions_flow(warns, None)?;
            interp.print_warnings();
            Ok(Step::More)
        }
        Status::Error(ref e) => {
            st.prompt_type = 1;
            let failure = parse_error_condition(source, e);
            interp.signal_parse_conditions_flow(warns, Some(failure))?;
            Ok(Step::More)
        }
        Status::Ok(ref exprs) => {
            interp.signal_parse_conditions_flow(warns, None)?;
            let expr = expr_to_value_src(&exprs[0], Some(("<text>", &text)));
            match parse_browser(interp, &expr, rho)? {
                1 => return Ok(Step::Leave),
                2 => return Ok(Step::More),
                _ => {}
            }
            // PR#15770 We don't want to step into expressions entered at
            // the debug prompt. The 'S' will be changed back to 's' after
            // the next eval.
            if interp.browser_last_command == b's' {
                interp.browser_last_command = b'S';
            }
            interp.visible = false;
            let value = interp.eval(&expr, rho)?;
            interp.base.clone().force_set(Sym::new(".Last.value"), value.clone());
            let visible = interp.visible;
            if visible {
                interp.print_value_toplevel(&value, rho)?;
            }
            interp.print_warnings();
            if !interp.task_callbacks.is_empty() {
                if let Some(f) = interp.call_toplevel_handlers(&expr, &value, visible) {
                    return Err(f);
                }
            }
            if interp.browser_last_command == b'S' {
                interp.browser_last_command = b's';
            }
            st.prompt_type = 1;
            Ok(Step::More)
        }
    }
}

/// `R_ReplConsole(rho, savestack, level)` inside `do_browser`, with the
/// browser's `CTXT_RESTART` context: an error at the prompt is reported
/// and the REPL starts over on the next line (R: main.c:do_browser's
/// `SETJMP(thiscontext.cjmpbuf)` and `R_InsertRestartHandlers`).
// R: main.c:do_browser, R_ReplConsole; errors.c:R_InsertRestartHandlers
pub(crate) fn browser_repl(interp: &mut Interp, rho: &Env, level: usize) -> RResult<()> {
    use crate::conditions::{HandlerEntry, HandlerNode, RestartNode};
    use std::rc::Rc;
    let saved_handlers = interp.handlers.clone();
    let saved_restarts = interp.restarts.clone();
    // The restart's exit: what `try_jump_to_restart` unwinds to.
    let exit = Env::new(None);
    let restart = Value::list(vec![Value::str("browser"), Value::Env(exit.clone())]);
    // The calling handler R_RestartToken: an error goes to default error
    // handling, which jumps to this browser, before outer handlers see it.
    let token: crate::conditions::RustHandler = Rc::new(|interp: &mut Interp, cond: &Value| {
        let (msg, call) = match cond {
            Value::List(l) => (
                match l.data.first() {
                    Some(Value::Str(m)) if m.data.len() == 1 => m.data[0].as_deref().unwrap_or("NA").to_string(),
                    _ => String::new(),
                },
                l.data.get(1).cloned().unwrap_or(Value::Null),
            ),
            _ => (String::new(), Value::Null),
        };
        Err(interp.default_error(&call, &msg))
    });
    let install = |interp: &mut Interp| {
        let entry = HandlerEntry {
            class: crate::value::RString::from("error"),
            parent_env: Value::Env(rho.clone()),
            handler: crate::conditions::rust_handler_value(token.clone()),
            target: None,
            calling: true,
        };
        interp.handlers = Some(Rc::new(HandlerNode { entry: Rc::new(entry), next: saved_handlers.clone() }));
        interp.restarts = Some(Rc::new(RestartNode { restart: restart.clone(), next: saved_restarts.clone() }));
    };
    let arena = robigo_syntax::Arena::new();
    let mut st = ReplState::new();
    install(interp);
    let frames = interp.frames.len();
    let result = loop {
        match browser_iteration(interp, rho, level, &mut st, &arena) {
            Ok(Step::More) => {}
            Ok(Step::Leave) => break Ok(()),
            Err(f) => {
                let f = match f {
                    Flow::Error(e) if !e.signaled => interp.signal_error(e),
                    f => f,
                };
                match f {
                    // The jump to this browser's restart: the error has
                    // been reported; R_ReplConsole starts over.
                    Flow::Return(_, ref env) if env.ptr_eq(&exit) => {
                        interp.frames.truncate(frames);
                        interp.visible = false;
                        st.reset();
                        install(interp);
                    }
                    f => break Err(f),
                }
            }
        }
    };
    interp.handlers = saved_handlers;
    interp.restarts = saved_restarts;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(buf: &str, end: usize) -> Vec<String> {
        robigo_syntax::parse_context(buf.as_bytes(), end)
    }

    #[test]
    fn parse_context_lines() {
        assert_eq!(context("parse error here", 11), ["parse error"]);
        assert_eq!(context("else 2", 4), ["else"]);
        // A newline at the end does not add an empty line.
        assert_eq!(context("if (TRUE) {\n", 12), ["if (TRUE) {"]);
        assert_eq!(context("a {\nxyz", 7), ["a {", "xyz"]);
    }

    #[test]
    fn parse_error_formats() {
        let e = robigo_syntax::ParseError::new("unexpected symbol", 6, 11);
        assert_eq!(parse_error_message("parse error here", &e), "unexpected symbol in \"parse error\"");
        let e2 = robigo_syntax::ParseError::new("unexpected 'else'", 0, 4);
        assert_eq!(parse_error_message("else 2", &e2), "unexpected 'else' in \"else\"");
        let e3 = robigo_syntax::ParseError::new("unexpected ')'", 14, 15);
        assert_eq!(parse_error_message("if (TRUE) {\n1\n)\n", &e3), "unexpected ')' in:\n\"1\n)\"");
        // A condition from `raiseParseError` keeps its position suffix.
        let e4 = robigo_syntax::ParseError::condition("repeatedFormal", "repeated formal argument 'x'", 5, 6, None);
        assert_eq!(parse_error_message("f(x, x)", &e4), "repeated formal argument 'x' (<input>:1:6)");
        // `error()` from a parser action has neither position nor context.
        let e5 = robigo_syntax::ParseError::r_error("attempt to use zero-length variable name");
        assert_eq!(parse_error_message("a[]", &e5), "attempt to use zero-length variable name");
    }

    /// A condition raised through `errorcall(call, ...)` keeps its call in
    /// the REPL (R: errors.c:verrorcall_dflt).
    #[test]
    fn parse_error_call_attribution() {
        let arena = robigo_syntax::Arena::new();
        let robigo_syntax::ConsoleStatus::Error(e, _) = robigo_syntax::parse_console(&arena, "1 |> f(_)\n") else {
            panic!("expected a parse error");
        };
        let ParseFailure::Condition { cond: Value::List(cond), call } = parse_error_condition("1 |> f(_)\n", &e) else {
            panic!("expected a parseError condition");
        };
        let Value::Str(msg) = &cond.data[0] else { panic!("condition message") };
        assert_eq!(
            crate::conditions::format_error_message(&call, msg.data[0].as_deref().unwrap()),
            "Error in f(\"_\") : \n  pipe placeholder can only be used as a named argument (<input>:1:6)\n"
        );
    }

    #[test]
    fn tab_expansion() {
        assert_eq!(context("a\there", 6), ["a       here"]);
    }

    /// The chunk statuses without the borrowed trees: what the REPL
    /// branches on.
    #[derive(PartialEq, Debug)]
    enum Kind {
        Null,
        Ok(usize),
        Incomplete,
        Error,
        Eof,
    }

    fn kind(buf: &str) -> (Kind, Vec<String>) {
        let arena = robigo_syntax::Arena::new();
        match parse_status(&arena, buf.into()) {
            (Status::Null, w) => (Kind::Null, w),
            (Status::Ok(es), w) => (Kind::Ok(es.len()), w),
            (Status::Incomplete, w) => (Kind::Incomplete, w),
            (Status::Error(_), w) => (Kind::Error, w),
            (Status::Eof, w) => (Kind::Eof, w),
        }
    }

    fn kind_bytes(raw: &[u8]) -> Kind {
        let arena = robigo_syntax::Arena::new();
        let text = robigo_syntax::lexable(raw);
        match parse_status(&arena, robigo_syntax::Source::new(&text, raw)).0 {
            Status::Null => Kind::Null,
            Status::Ok(es) => Kind::Ok(es.len()),
            Status::Incomplete => Kind::Incomplete,
            Status::Error(_) => Kind::Error,
            Status::Eof => Kind::Eof,
        }
    }

    /// gram.y:mbcs_get_next returns END_OF_INPUT when a character's bytes
    /// run out: at the start of a chunk the session ends quietly (Rscript
    /// exits 0), after an expression the chunk is incomplete; enough bytes
    /// that do not form a character are "invalid multibyte character".
    #[test]
    fn invalid_bytes_end_input_or_error() {
        assert_eq!(kind_bytes(b"\xff yy\n"), Kind::Eof);
        assert_eq!(kind_bytes(b"zz \xff\n"), Kind::Incomplete);
        assert_eq!(kind_bytes(b"zz \xff yy + 1 + 2\n"), Kind::Error);
        assert_eq!(kind_bytes("x + \u{20ac}\n".as_bytes()), Kind::Error);
    }

    #[test]
    fn lone_semicolon_is_an_error() {
        // `;` alone is PARSE_ERROR ("unexpected ';'"), not PARSE_NULL.
        assert_eq!(kind(";\n").0, Kind::Error);
        assert_eq!(kind(" ;").0, Kind::Error);
        // Blank and comment-only lines are PARSE_NULL.
        assert_eq!(kind("\n").0, Kind::Null);
        assert_eq!(kind("   \n").0, Kind::Null);
        assert_eq!(kind("# c\n").0, Kind::Null);
    }

    #[test]
    fn terminated_statements_are_ok() {
        assert_eq!(kind("1 + 1\n").0, Kind::Ok(1));
        assert_eq!(kind("x <- 1;").0, Kind::Ok(1));
        // Multi-expression chunks cannot occur from the REPL's copy loop;
        // single chunks with a trailing separator parse as one.
        assert_eq!(kind("1;\n").0, Kind::Ok(1));
    }

    #[test]
    fn unterminated_statements_are_incomplete() {
        assert_eq!(kind("if (TRUE) {\n").0, Kind::Incomplete);
        assert_eq!(kind("1 +\n").0, Kind::Incomplete);
        assert_eq!(kind("\"abc\n").0, Kind::Incomplete);
        assert_eq!(kind("if (x) 1 else\n").0, Kind::Incomplete);
        // A line longer than the console buffer is cut mid-line: a chunk
        // without its terminating `;` or newline needs more input.
        assert_eq!(kind("1 + 1 + ").0, Kind::Incomplete);
    }

    #[test]
    fn comments_cut_by_a_semicolon_are_incomplete() {
        // The REPL's cut lands inside the comment; R's lexer sets
        // EndOfFile = 2 there, whatever the grammar makes of the buffer.
        assert_eq!(kind("# c;").0, Kind::Incomplete);
        assert_eq!(kind("1 # two;").0, Kind::Incomplete);
        assert_eq!(kind("f({\n1 # c;").0, Kind::Incomplete);
        // A comment closed by a newline is fine.
        assert_eq!(kind("# c\n").0, Kind::Null);
        assert_eq!(kind("f({\n# c;\n").0, Kind::Incomplete);
        // Strings and quoted names hide a `#`.
        assert_eq!(kind("\"a#b\";").0, Kind::Ok(1));
        assert_eq!(kind("`a#b`;\n").0, Kind::Ok(1));
    }
}
