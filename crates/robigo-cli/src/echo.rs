//! R's non-interactive console: how `R -f FILE` reads a file as console
//! input and prints the transcript that `R CMD check` compares. `Rscript
//! FILE` is the same loop with prompt and echo suppressed (`--no-echo`),
//! and `R -e EXPR` feeds the expressions through it as a temporary file.
//!
//! Ports:
//! - The console stream itself (`R: unix/sys-std.c:Rstd_ReadConsole`:
//!   prompt, echo, line reads) is `robigo_runtime::console`, which the
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

use robigo_runtime::builtins::parsing::ParseFailure;
use robigo_runtime::lang::expr_to_value_src;
use robigo_runtime::value::{TRUE, Value};
use robigo_runtime::{Interp, Sym};

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
pub(crate) fn prompt_string(interp: &Interp, echo: bool, continue_line: bool) -> String {
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
    robigo_runtime::console::read_console(prompt)
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
            robigo_runtime::builtins::parsing::parse_condition(buf, robigo_syntax::CONSOLE_SRCNAME, e)
        }
        _ => ParseFailure::Error { call: Some(Value::Null), message: parse_error_message(buf, e) },
    }
}

/// Did the session set an error handler that keeps it alive after an
/// error (R: main.c:check_session_exit: a non-NULL `error` option or
/// `catch.script.errors = TRUE`)?
fn catches_script_errors(interp: &Interp) -> bool {
    interp.get_option("error").map(|o| !o.is_null()).unwrap_or(false)
        || robigo_runtime::coerce::as_logical(&interp.get_option("catch.script.errors").unwrap_or(Value::Null)) == TRUE
}

/// `R_ReplConsole` over the console: read a line, echo it, parse and
/// evaluate one statement at a time until end of input or an error.
/// Returns `false` when the session must halt with "Execution halted"
/// (whatever failed has been written already).
// R: main.c:R_ReplConsole, Rf_ReplIteration, end_Rmainloop
pub fn repl_console(interp: &mut Interp, echo: bool) -> bool {
    let mut line: Vec<u8> = Vec::new(); // state->buf
    let mut bufp = 0; // state->bufp
    let mut iob: Vec<u8> = Vec::new(); // R_ConsoleIob
    let mut prompt_type = 1; // 1: `> `, 2: `+ `
    let mut status_incomplete = false; // state->status == PARSE_INCOMPLETE
    let global = interp.global.clone();
    let arena = robigo_syntax::Arena::new();
    loop {
        if bufp >= line.len() {
            // R_ReadConsole: buffered output goes out before the prompt.
            interp.flush();
            let prompt = prompt_string(interp, echo, prompt_type == 2);
            match read_console(&prompt) {
                Some(l) => {
                    line = l;
                    bufp = 0;
                }
                None => {
                    // R_ReplConsole: end of input while a statement is
                    // still open is an error.
                    if status_incomplete {
                        if !interp.signal_parse_conditions(
                            Vec::new(),
                            Some(ParseFailure::Error {
                                call: Some(Value::Null),
                                message: "unexpected end of input".into(),
                            }),
                        ) && !catches_script_errors(interp)
                        {
                            return false;
                        }
                        // check_session_exit continued: the restarted
                        // REPL reads the end of input again, with no
                        // statement open.
                        status_incomplete = false;
                        iob.clear();
                        prompt_type = 1;
                        continue;
                    }
                    // end_Rmainloop: `Rprintf("\n")`, except in no-echo mode.
                    if echo {
                        interp.write_out("\n");
                        interp.flush();
                    }
                    return true;
                }
            }
        }
        // Feed the console buffer up to the first `;` or newline, like
        // Rf_ReplIteration's copy loop (which does not look inside
        // strings or comments).
        while bufp < line.len() {
            let c = line[bufp];
            bufp += 1;
            iob.push(c);
            if c == b';' || c == b'\n' {
                break;
            }
        }
        // The lexer reads `text`, the chunk with each invalid UTF-8 byte
        // replaced (robigo_syntax::Source); errors and source references
        // are reported against it.
        let text = robigo_syntax::lexable(&iob);
        let source = robigo_syntax::Source::new(&text, &iob);
        let (st, warns) = parse_status(&arena, source);
        status_incomplete = matches!(st, Status::Incomplete);
        // The statement is done with (R resets R_ConsoleIob); cleared once
        // `text`, which may borrow it, is gone.
        let mut done = false;
        let ok = match st {
            Status::Null => {
                done = true;
                prompt_type = 1;
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
                prompt_type = 2;
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
                    prompt_type = 1;
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
            iob.clear();
        }
        if !ok {
            // The error longjmped to the top level, whatever raised it.
            if !catches_script_errors(interp) {
                return false;
            }
            // check_session_exit continued the session: R's longjmp lands
            // in R_ReplConsole's caller, which restarts the REPL — the
            // statement and the rest of the current line are dropped and
            // reading continues at the next one.
            line.clear();
            bufp = 0;
            iob.clear();
            prompt_type = 1;
            status_incomplete = false;
        }
    }
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
            robigo_runtime::conditions::format_error_message(&call, msg.data[0].as_deref().unwrap()),
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
