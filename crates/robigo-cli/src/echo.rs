//! R's non-interactive console: how `R -f FILE` reads a file as console
//! input and prints the transcript that `R CMD check` compares. `Rscript
//! FILE` is the same loop with prompt and echo suppressed (`--no-echo`),
//! and `R -e EXPR` feeds the expressions through it as a temporary file.
//!
//! Ports:
//! - `R: unix/sys-std.c:Rstd_ReadConsole` (the `!R_Interactive` branch):
//!   the prompt, the echo of each line exactly as read, the newline
//!   appended to a final line without one, and reads capped at
//!   `CONSOLE_BUFFER_SIZE` bytes.
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

use std::io::Read;

use robigo_runtime::lang::expr_to_value_src;
use robigo_runtime::value::{TRUE, Value};
use robigo_runtime::{Interp, Sym};

/// `CONSOLE_BUFFER_SIZE` (R: src/include/Defn.h): reads return at most
/// this many bytes, so longer lines are read (and echoed) in pieces.
pub const CONSOLE_BUFFER_SIZE: usize = 4096;

// R: gram.y / Defn.h: PARSE_CONTEXT_SIZE (circular buffer, in bytes).
const PARSE_CONTEXT_SIZE: usize = 256;

/// The parse statuses of `R_Parse1Buffer` the REPL distinguishes (R:
/// main.c:Rf_ReplIteration's switch over `state->status`).
enum Status<'a> {
    Null,
    Ok(Vec<robigo_syntax::Expr<'a>>),
    Incomplete,
    Error(robigo_syntax::ParseError),
}

/// Console input with one byte of pushback, like the `FILE *` R reads
/// from (`fgets` peeks at the next byte only through the stream's own
/// buffer, which `feof` and the final-newline rule depend on).
pub struct ConsoleInput {
    inner: Box<dyn Read>,
    pending: Option<u8>,
}

impl ConsoleInput {
    pub fn new(inner: Box<dyn Read>) -> ConsoleInput {
        ConsoleInput { inner, pending: None }
    }

    fn getc(&mut self) -> Option<u8> {
        if let Some(b) = self.pending.take() {
            return Some(b);
        }
        let mut byte = [0u8; 1];
        match self.inner.read(&mut byte) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(byte[0]),
        }
    }

    fn ungetc(&mut self, b: u8) {
        self.pending = Some(b);
    }
}

/// Like `R_PromptString` (R: main.c): the `prompt` or `continue` option
/// (re-read every line, so R code can change them), or nothing in
/// no-echo mode.
fn prompt_string(interp: &Interp, echo: bool, continue_line: bool) -> String {
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

/// A chunk written to the console: like R's `fputs` to `stdout` (not via
/// the connection machinery, which `Rprintf` would use; prompts and echo
/// bypass `sink()`).
fn out(s: &str) {
    use std::io::Write;
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(s.as_bytes());
    let _ = stdout.flush();
}

/// One `R_ReadConsole` call (R: unix/sys-std.c:Rstd_ReadConsole, the
/// non-interactive branch): the prompt is printed and flushed, a line is
/// read and echoed. Returns `None` on end of input.
fn read_console(input: &mut ConsoleInput, echo: bool, prompt: &str) -> Option<String> {
    if echo {
        out(prompt);
    }
    // `fgets(buf, CONSOLE_BUFFER_SIZE, ifp)`: up to 4095 bytes, stopping
    // after a newline.
    let mut bytes: Vec<u8> = Vec::new();
    while bytes.len() < CONSOLE_BUFFER_SIZE - 1 {
        let Some(b) = input.getc() else { break };
        bytes.push(b);
        if b == b'\n' {
            break;
        }
    }
    if bytes.is_empty() {
        return None;
    }
    // Remove CR in CRLF ending.
    if bytes.len() >= 2 && bytes.ends_with(b"\r\n") {
        let n = bytes.len();
        bytes.truncate(n - 2);
        bytes.push(b'\n');
    }
    // At end of input, a line not terminated by `\n` gets one (so the
    // transcript ends with a newline).
    if !bytes.ends_with(b"\n") {
        if let Some(b) = input.getc() {
            input.ungetc(b);
        } else {
            bytes.push(b'\n');
        }
    }
    let line = String::from_utf8_lossy(&bytes).into_owned();
    if echo {
        out(&line);
    }
    Some(line)
}

/// `R_Parse1Buffer` with `gend = 0` (R: gram.c:R_Parse1 over the grammar
/// `prog: END_OF_INPUT | '\n' | expr '\n' | expr ';' | error`): does the
/// buffer hold a complete statement, is it incomplete, or is it an error?
/// The REPL feeds the buffer up to the first `;` or newline, so a
/// complete buffer is always one terminated statement. The lexer's
/// warnings are returned for the caller to signal, as R's lexer does
/// while building the expression (`GenerateCode` gates them to this one
/// parse).
fn parse_status<'a>(arena: &'a robigo_syntax::Arena, buf: &'a str) -> (Status<'a>, Vec<String>) {
    match robigo_syntax::parse_console(arena, buf) {
        robigo_syntax::ConsoleStatus::Null => (Status::Null, Vec::new()),
        robigo_syntax::ConsoleStatus::Ok(exprs, warnings) => (Status::Ok(exprs), warnings),
        robigo_syntax::ConsoleStatus::Incomplete => (Status::Incomplete, Vec::new()),
        robigo_syntax::ConsoleStatus::Error(e) => (Status::Error(e), Vec::new()),
        // The REPL's chunks are never empty (a read returns at least one
        // byte); an empty buffer would just be a null statement.
        robigo_syntax::ConsoleStatus::Eof => (Status::Null, Vec::new()),
    }
}

fn floor_char_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// The last `PARSE_CONTEXT_SIZE` characters before `end`, tab-expanded
/// like `tabExpand` and split into lines like `getParseContext` (R:
/// main/source.c), which also drops an empty line after a newline. R's
/// ring buffer works in bytes, so a slice can split a multi-byte
/// character; the pieces are shown lossily instead of panicking.
fn parse_context(buf: &str, end: usize) -> Vec<String> {
    let end = end.min(buf.len());
    let start = floor_char_boundary(buf, end.saturating_sub(PARSE_CONTEXT_SIZE));
    // R: source.c:getParseContext / tabExpand. Expand each line in its
    // own 200-byte buffer, stopping input once 192 bytes are written.
    let mut lines: Vec<String> = buf.as_bytes()[start..end]
        .split(|b| *b == b'\n')
        .map(|line| {
            let mut expanded = Vec::new();
            for &b in line {
                if expanded.len() >= 192 {
                    break;
                }
                if b == b'\t' {
                    loop {
                        expanded.push(b' ');
                        if expanded.len() % 8 == 0 {
                            break;
                        }
                    }
                } else {
                    expanded.push(b);
                }
            }
            String::from_utf8_lossy(&expanded).into_owned()
        })
        .collect();
    if let Some(last) = lines.last() {
        if last.is_empty() {
            lines.pop();
        }
    }
    lines
}

/// A parse error as the REPL reports it. Only grammar errors (`yyerror`)
/// reach `parseError(R_NilValue, 0)` (R: main/source.c), which appends the
/// last lines of the parse context; conditions raised by `raiseParseError`
/// and `raiseLexError` carry their own `(<input>:<line>:<col>)` suffix,
/// and a plain `error()` from a parser action (`names.c:install`) has no
/// position or context at all.
fn parse_error_message(buf: &str, e: &robigo_syntax::ParseError) -> String {
    use robigo_syntax::ErrorStyle;
    match e.style {
        ErrorStyle::Condition => robigo_syntax::condition_message(buf, robigo_syntax::CONSOLE_SRCNAME, e),
        ErrorStyle::RError => e.message.clone(),
        ErrorStyle::Grammar => {
            let lines = parse_context(buf, e.end);
            match lines.len() {
                0 => e.message.clone(),
                1 => format!("{} in \"{}\"", e.message, lines[0]),
                _ => format!("{} in:\n\"{}\n{}\"", e.message, lines[lines.len() - 2], lines[lines.len() - 1]),
            }
        }
    }
}

/// The whole `Error...` line the REPL prints. A condition raised through
/// `errorcall(call, ...)` keeps its call attribution even in the REPL
/// (R: errors.c:verrorcall_dflt; gram.y:R_Parse1 passes `R_CurrentExpr`
/// for the pipe checks), so `1 |> f(_)` reports `Error in f("_") : ...`.
fn parse_error_line(buf: &str, e: &robigo_syntax::ParseError) -> String {
    match (&e.style, &e.call) {
        (robigo_syntax::ErrorStyle::Condition, Some(call)) => robigo_runtime::conditions::format_error_message(
            &robigo_runtime::builtins::parsing::error_expr_value(call),
            &parse_error_message(buf, e),
        ),
        _ => format!("Error: {}\n", parse_error_message(buf, e)),
    }
}

/// Did the session set an error handler that keeps it alive after an
/// error (R: main.c:check_session_exit: a non-NULL `error` option or
/// `catch.script.errors = TRUE`)?
fn catches_script_errors(interp: &Interp) -> bool {
    interp.get_option("error").map(|o| !o.is_null()).unwrap_or(false)
        || robigo_runtime::coerce::as_logical(&interp.get_option("catch.script.errors").unwrap_or(Value::Null)) == TRUE
}

/// `R_ReplConsole` over `input`: read a line, echo it, parse and
/// evaluate one statement at a time until end of input or an error.
/// Returns `false` when the session must halt with "Execution halted"
/// (whatever failed has been written already).
// R: main.c:R_ReplConsole, Rf_ReplIteration, end_Rmainloop
pub fn repl_console(interp: &mut Interp, input: &mut ConsoleInput, echo: bool) -> bool {
    let mut line = String::new(); // state->buf
    let mut bufp = 0; // state->bufp
    let mut iob = String::new(); // R_ConsoleIob
    let mut prompt_type = 1; // 1: `> `, 2: `+ `
    let mut status_incomplete = false; // state->status == PARSE_INCOMPLETE
    let global = interp.global.clone();
    let arena = robigo_syntax::Arena::new();
    loop {
        if bufp >= line.len() {
            // R_ReadConsole: buffered output goes out before the prompt.
            interp.flush();
            let prompt = prompt_string(interp, echo, prompt_type == 2);
            match read_console(input, echo, &prompt) {
                Some(l) => {
                    line = l;
                    bufp = 0;
                }
                None => {
                    // R_ReplConsole: end of input while a statement is
                    // still open is an error.
                    if status_incomplete {
                        interp.write_err("Error: unexpected end of input\n");
                        return false;
                    }
                    // end_Rmainloop: no trailing newline in no-echo mode.
                    if echo {
                        out("\n");
                    }
                    return true;
                }
            }
        }
        // Feed the console buffer up to the first `;` or newline, like
        // Rf_ReplIteration's copy loop (which does not look inside
        // strings or comments).
        while bufp < line.len() {
            let c = line[bufp..].chars().next().unwrap();
            bufp += c.len_utf8();
            iob.push(c);
            if c == ';' || c == '\n' {
                break;
            }
        }
        let (st, warns) = parse_status(&arena, &iob);
        status_incomplete = matches!(st, Status::Incomplete);
        match st {
            Status::Null => {
                iob.clear();
                prompt_type = 1;
            }
            Status::Incomplete => prompt_type = 2,
            Status::Error(ref e) => {
                interp.write_err(&parse_error_line(&iob, e));
                return false;
            }
            Status::Ok(ref exprs) => {
                // The real parse (R's `gencode = 1`) signals the lexer's
                // warnings while building the expression; with
                // `options(warn = 2)` they are errors and stop the
                // script before it evaluates anything.
                let mut parse_failed = None;
                for w in &warns {
                    if let Err(f) = interp.warningcall(&Value::Null, w.clone()) {
                        parse_failed = Some(f);
                        break;
                    }
                }
                if let Some(f) = parse_failed {
                    interp.report_toplevel(f);
                    interp.print_warnings();
                    return false;
                }
                // Rf_ReplIteration's PARSE_OK branch.
                let v = expr_to_value_src(&exprs[0], Some(("<text>", &iob)));
                let value = interp.eval_toplevel(&v, &global, true);
                if let Some(v) = &value {
                    // SET_SYMVALUE(R_LastvalueSymbol, value): a binding in
                    // the base environment, not the global one
                    interp.base.clone().force_set(Sym::new(".Last.value"), v.clone());
                }
                // Rf_ReplIteration's PARSE_OK branch ends with
                // Rf_callToplevelHandlers(lastExpr, value, TRUE, wasDisplayed);
                // an evaluation error longjmps past it.
                if let Some(val) = &value {
                    if !interp.task_callbacks.is_empty() {
                        let visible = interp.toplevel_visible;
                        // A warning converted to an error (options(warn = 2))
                        // longjmps out of Rf_callToplevelHandlers to the top
                        // level, like an evaluation error.
                        if let Some(f) = interp.call_toplevel_handlers(&v, val, visible) {
                            interp.report_toplevel(f);
                            return false;
                        }
                    }
                }
                iob.clear();
                prompt_type = 1;
                if value.is_none() && catches_script_errors(interp) {
                    // check_session_exit continued the session: R's
                    // longjmp lands in R_ReplConsole's caller, which
                    // restarts the REPL — the rest of the current line is
                    // dropped and reading continues at the next one.
                    line.clear();
                    bufp = 0;
                    iob.clear();
                    prompt_type = 1;
                } else if value.is_none() {
                    return false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(buf: &str, end: usize) -> Vec<String> {
        parse_context(buf, end)
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
        let e4 = robigo_syntax::ParseError::condition("repeated formal argument 'x'", 5, 6, None);
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
        let robigo_syntax::ConsoleStatus::Error(e) = robigo_syntax::parse_console(&arena, "1 |> f(_)\n") else {
            panic!("expected a parse error");
        };
        assert_eq!(
            parse_error_line("1 |> f(_)\n", &e),
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
    }

    fn kind(buf: &str) -> (Kind, Vec<String>) {
        let arena = robigo_syntax::Arena::new();
        match parse_status(&arena, buf) {
            (Status::Null, w) => (Kind::Null, w),
            (Status::Ok(es), w) => (Kind::Ok(es.len()), w),
            (Status::Incomplete, w) => (Kind::Incomplete, w),
            (Status::Error(_), w) => (Kind::Error, w),
        }
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
