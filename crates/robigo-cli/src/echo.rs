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

/// `PARSE_CONTEXT_SIZE` (R: gram.c): the parse error context holds the
/// last this many characters.
const PARSE_CONTEXT_SIZE: usize = 50;

/// The parse statuses of `R_Parse1Buffer` the REPL distinguishes (R:
/// main.c:Rf_ReplIteration's switch over `state->status`).
enum Status {
    Null,
    Ok(Vec<robigo_syntax::Expr>),
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
fn parse_status(buf: &str) -> (Status, Vec<String>) {
    // R's lexer notes when the buffer ends inside a comment it cannot
    // close (R: gram.y's SkipComment sets EndOfFile = 2); every parse
    // status is then PARSE_INCOMPLETE, whatever the grammar says. The
    // REPL reaches this whenever its raw `;`/newline cut lands inside a
    // comment (a `;` in a comment, say).
    if ends_in_comment(buf) {
        return (Status::Incomplete, Vec::new());
    }
    match robigo_syntax::parse_program_with_warnings(buf) {
        Err(e) if e.incomplete => (Status::Incomplete, Vec::new()),
        Err(e) => (Status::Error(e), Vec::new()),
        Ok((exprs, warnings)) => {
            // R's grammar only accepts a statement that is TERMINATED
            // (`prog: '\n' | expr ';' | expr '\n'`); the REPL's own
            // chunks are, except when a line longer than
            // `CONSOLE_BUFFER_SIZE` is cut mid-line (R: PARSE_EOF from
            // `R_Parse1`, which `R_ReplConsole` treats as more input
            // pending).
            if !buf.ends_with(';') && !buf.ends_with('\n') {
                return (Status::Incomplete, warnings);
            }
            if exprs.is_empty() {
                // The grammar has no rule that can start a statement with
                // a `;` (`1;;2` is an error at the second one);
                // robigo-syntax's file parser skips separators between
                // expressions, so a chunk that is only a separator is
                // detected here, from the first token. Any other chunk
                // that parses to nothing (blank, comment) is `'\n'`:
                // PARSE_NULL.
                let t = first_semicolon(buf);
                if t.0 < buf.len() {
                    return (
                        Status::Error(robigo_syntax::ParseError {
                            message: "unexpected ';'".to_string(),
                            offset: t.0,
                            end: t.1,
                            incomplete: false,
                        }),
                        warnings,
                    );
                }
                return (Status::Null, warnings);
            }
            (Status::Ok(exprs), warnings)
        }
    }
}

/// The byte range of `buf`'s first token if it is a `;` (empty range
/// when it is not; see `parse_status`).
fn first_semicolon(buf: &str) -> (usize, usize) {
    match robigo_syntax::lexer::Lexer::new(buf).tokenize() {
        Ok((tokens, _)) => match tokens.first() {
            Some(t) if matches!(t.tok, robigo_syntax::lexer::Tok::Semi) => (t.span.start as usize, t.span.end as usize),
            _ => (buf.len(), buf.len()),
        },
        Err(_) => (buf.len(), buf.len()),
    }
}

/// Does `buf` end inside an unclosed comment? A comment runs from a
/// `#` (outside strings and quoted names) to the end of the line, so
/// only the text after the last newline can hold one; quotes opened
/// before it imply an unterminated string, which the parser already
/// reports as incomplete.
fn ends_in_comment(buf: &str) -> bool {
    let tail = match buf.rfind('\n') {
        Some(i) => &buf[i + 1..],
        None => buf,
    };
    let b = tail.as_bytes();
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match quote {
            Some(q) => {
                if c == b'\\' {
                    i += 1; // an escaped character, possibly a quote
                } else if c == q {
                    quote = None;
                }
            }
            None => {
                if c == b'\'' || c == b'"' || c == b'`' {
                    quote = Some(c);
                } else if c == b'#' {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
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
    let mut context = String::new();
    for c in String::from_utf8_lossy(&buf.as_bytes()[start..end]).chars() {
        if c == '\t' {
            // Fill with spaces to the next multiple of 8.
            loop {
                context.push(' ');
                if context.len() % 8 == 0 {
                    break;
                }
            }
        } else {
            context.push(c);
        }
    }
    let mut lines: Vec<String> = context.split('\n').map(str::to_string).collect();
    if let Some(last) = lines.last() {
        if last.is_empty() {
            lines.pop();
        }
    }
    lines
}

/// A parse error as `parseError(R_NilValue, 0)` reports it (R:
/// main/source.c): plain, with one line of context, or with the last
/// two lines.
fn parse_error_message(buf: &str, e: &robigo_syntax::ParseError) -> String {
    let lines = parse_context(buf, e.end);
    match lines.len() {
        0 => e.message.clone(),
        1 => format!("{} in \"{}\"", e.message, lines[0]),
        _ => format!("{} in:\n\"{}\n{}\"", e.message, lines[lines.len() - 2], lines[lines.len() - 1]),
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
    let mut status = Status::Null; // state->status
    let global = interp.global.clone();
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
                    if matches!(status, Status::Incomplete) {
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
        let (st, warns) = parse_status(&iob);
        status = st;
        match status {
            Status::Null => {
                iob.clear();
                prompt_type = 1;
            }
            Status::Incomplete => prompt_type = 2,
            Status::Error(ref e) => {
                let msg = parse_error_message(&iob, e);
                interp.write_err(&format!("Error: {}\n", msg));
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
                    // SET_SYMVALUE(R_LastvalueSymbol, value)
                    global.force_set(Sym::new(".Last.value"), v.clone());
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
        let e =
            robigo_syntax::ParseError { message: "unexpected symbol".into(), offset: 6, end: 11, incomplete: false };
        assert_eq!(parse_error_message("parse error here", &e), "unexpected symbol in \"parse error\"");
        let e2 =
            robigo_syntax::ParseError { message: "unexpected 'else'".into(), offset: 0, end: 4, incomplete: false };
        assert_eq!(parse_error_message("else 2", &e2), "unexpected 'else' in \"else\"");
        let e3 = robigo_syntax::ParseError { message: "unexpected ')'".into(), offset: 14, end: 15, incomplete: false };
        assert_eq!(parse_error_message("if (TRUE) {\n1\n)\n", &e3), "unexpected ')' in:\n\"1\n)\"");
    }

    #[test]
    fn tab_expansion() {
        assert_eq!(context("a\there", 6), ["a       here"]);
    }

    #[test]
    fn lone_semicolon_is_an_error() {
        // `;` alone is PARSE_ERROR ("unexpected ';'"), not PARSE_NULL.
        assert!(matches!(parse_status(";\n"), (Status::Error(_), _)));
        assert!(matches!(parse_status(" ;"), (Status::Error(_), _)));
        // Blank and comment-only lines are PARSE_NULL.
        assert!(matches!(parse_status("\n"), (Status::Null, _)));
        assert!(matches!(parse_status("   \n"), (Status::Null, _)));
        assert!(matches!(parse_status("# c\n"), (Status::Null, _)));
    }

    #[test]
    fn terminated_statements_are_ok() {
        assert!(matches!(parse_status("1 + 1\n"), (Status::Ok(_), _)));
        assert!(matches!(parse_status("x <- 1;"), (Status::Ok(_), _)));
        // Multi-expression chunks cannot occur from the REPL's copy loop;
        // single chunks with a trailing separator parse as one.
        let (st, _) = parse_status("1;\n");
        assert!(matches!(st, Status::Ok(es) if es.len() == 1));
    }

    #[test]
    fn unterminated_statements_are_incomplete() {
        assert!(matches!(parse_status("if (TRUE) {\n"), (Status::Incomplete, _)));
        assert!(matches!(parse_status("1 +\n"), (Status::Incomplete, _)));
        assert!(matches!(parse_status("\"abc\n"), (Status::Incomplete, _)));
        assert!(matches!(parse_status("if (x) 1 else\n"), (Status::Incomplete, _)));
        // A line longer than the console buffer is cut mid-line: a chunk
        // without its terminating `;` or newline needs more input.
        assert!(matches!(parse_status("1 + 1 + "), (Status::Incomplete, _)));
    }

    #[test]
    fn comments_cut_by_a_semicolon_are_incomplete() {
        // The REPL's cut lands inside the comment; R's lexer sets
        // EndOfFile = 2 there, whatever the grammar makes of the buffer.
        assert!(matches!(parse_status("# c;"), (Status::Incomplete, _)));
        assert!(matches!(parse_status("1 # two;"), (Status::Incomplete, _)));
        assert!(matches!(parse_status("f({\n1 # c;"), (Status::Incomplete, _)));
        // A comment closed by a newline is fine.
        assert!(matches!(parse_status("# c\n"), (Status::Null, _)));
        assert!(matches!(parse_status("f({\n# c;\n"), (Status::Incomplete, _)));
        // Strings and quoted names hide a `#`.
        assert!(matches!(parse_status("\"a#b\";"), (Status::Ok(_), _)));
        assert!(matches!(parse_status("`a#b`;\n"), (Status::Ok(_), _)));
    }
}
