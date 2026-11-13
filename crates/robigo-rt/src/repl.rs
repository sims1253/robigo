//! Rscript's top level over robigo-rt: `R --no-echo --file=FILE` reads the
//! file as console input, one statement at a time (`R_ReplConsole`,
//! `Rf_ReplIteration`), evaluates each in the global environment, prints a
//! visible value, then the deferred warnings; an error is reported and ends
//! the session ("Execution halted", exit status 1: `check_session_exit`
//! with no `options(error)`). `Rscript -e EXPR` is the same loop over the
//! expressions, one per line.
//!
//! Stand-ins until their owners land, each saying where it stops:
//! - the error report is `verrorcall_dflt`'s text without
//!   `show.error.locations` and without the "Calls:" traceback, written
//!   when the error reaches the top level (S1 reports at the error, before
//!   `on.exit` code runs, as GNU R does);
//! - a call in an error or warning is deparsed by [`deparse_call_line`],
//!   which covers calls of symbols and scalar constants on one line of at
//!   most 60 bytes and says "not implemented" for anything else (B3's
//!   deparse replaces it);
//! - printing is [`crate::print`]'s.

use robigo_heap::types::*;
use robigo_rtcore::{Cx, Lgl, RError, Ref};
use std::io::Write;

/// `LONGWARN`: the width at which an error or warning moves its message to
/// the next line.
const LONGWARN: usize = 75;
/// `R_WarnLength`: `options(warning.length)`'s default.
const WARN_LENGTH: usize = 1000;
/// `DEFAULT_Cutoff`: deparse's line width.
const CUTOFF: usize = 60;

/// Where the session writes: GNU R's console (`Rprintf`) and its error
/// stream (`REprintf`). Errors flush the console first, so a terminal or
/// a merged pipe sees GNU R's order.
pub struct Io<'w> {
    pub out: &'w mut dyn Write,
    pub err: &'w mut dyn Write,
}

impl Io<'_> {
    fn out(&mut self, b: &[u8]) {
        let _ = self.out.write_all(b);
    }
    fn err(&mut self, b: &[u8]) {
        let _ = self.out.flush();
        let _ = self.err.write_all(b);
        let _ = self.err.flush();
    }
}

/// Run `src` as Rscript runs a script file: true when it ran to the end,
/// false when an error ended it (the error written already; the caller
/// writes "Execution halted" and exits with status 1).
// R: main.c:R_ReplConsole, Rf_ReplIteration; unix/sys-std.c:Rstd_ReadConsole
pub fn run_console(cx: &mut Cx, src: &[u8], io: &mut Io<'_>) -> bool {
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
                        return parse_warnings_unsupported(io);
                    }
                    incomplete = true;
                    continue;
                }
                robigo_syntax::ConsoleStatus::Error(e, warnings) => {
                    if !warnings.is_empty() {
                        return parse_warnings_unsupported(io);
                    }
                    let msg = parse_error_message(source, &e);
                    io.err(format!("Error: {msg}\n").as_bytes());
                    false
                }
                robigo_syntax::ConsoleStatus::Ok(exprs, warnings) => {
                    if !warnings.is_empty() {
                        return parse_warnings_unsupported(io);
                    }
                    eval_statement(cx, &exprs[0], io)
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
        io.err(b"Error: unexpected end of input\n");
        return false;
    }
    true
}

/// The lexer's warnings (signalled as R warnings by GNU R's parse) need
/// the condition system to be raised at the top level.
fn parse_warnings_unsupported(io: &mut Io<'_>) -> bool {
    io.err(b"Error: robigo: the parser's warnings at the top level are not implemented yet\n");
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
fn eval_statement(cx: &mut Cx, expr: &robigo_syntax::Expr<'_>, io: &mut Io<'_>) -> bool {
    cx.scope(|cx, s| {
        let e = s.out(cx);
        let er = e.root();
        if let Err(err) = crate::reader::lower_one(cx, expr, e) {
            let text = error_text(cx, &err);
            cx.resolve_exit(err, None);
            io.err(text.as_bytes());
            return false;
        }
        let g = s.root(cx, cx.global_env());
        let base = s.root(cx, cx.base_env());
        let last = s.root(cx, cx.nil());
        let last_sym = s.out(cx);
        let last_sym_r = last_sym.root();
        if cx.build(last_sym, |cx| Ok(cx.install(b".Last.value")?.as_ref())).is_err() {
            unreachable!("robigo: interning a symbol does not error");
        }
        let o = s.out(cx);
        let or = o.root();
        let mut failure = None;
        let mut printed = Vec::new();
        let done = cx
            .toplevel(g, base, |cx| {
                cx.set_visible(false);
                let r = crate::eval(cx, er, g, o).and_then(|()| {
                    // SET_SYMVALUE(R_LastvalueSymbol, value)
                    let _ = last;
                    cx.define_var(last_sym_r, or, base)?;
                    if cx.visible() {
                        crate::print::print_value_env(cx, or, &mut printed)?;
                    }
                    Ok(())
                });
                if let Err(err) = &r {
                    failure = Some(error_text(cx, err));
                }
                r
            })
            .expect("robigo: an exit that passed the top level");
        io.out(&printed);
        if !done {
            // verrorcall_dflt: the message, then the deferred warnings
            io.err(failure.unwrap_or_default().as_bytes());
            if !cx.collected_warnings().is_empty() {
                io.err(b"In addition: ");
                print_warnings(cx, io);
            }
            return false;
        }
        print_warnings(cx, io);
        true
    })
}

/// The text `verrorcall_dflt` writes for the pending error `e` (its message
/// and call), without the source location and traceback (S1's).
// R: errors.c:verrorcall_dflt
pub fn error_text(cx: &Cx, e: &RError) -> String {
    let (v, _, call) = cx.exit_objects(e);
    let msg = match v.as_vector::<robigo_rtcore::Char<'_>>() {
        Some(s) if s.len() == 1 => String::from_utf8_lossy(s.get(0).bytes()).into_owned(),
        _ => format!("robigo: an exit of kind {:?} reached the top level", cx.exit_info(e)),
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

/// `PrintWarnings()`: the deferred warnings, then the list cleared.
/// (`last.warning` is not set: nothing reads it before S1.)
// R: errors.c:PrintWarnings
pub fn print_warnings(cx: &mut Cx, io: &mut Io<'_>) {
    let ws = cx.collected_warnings();
    let n = ws.len();
    if n == 0 {
        return;
    }
    let mut out = String::new();
    let item = |call: Ref<'_>, msg: &str, prefix: &str, indent: usize| -> String {
        if call.sexptype() == NILSXP {
            return format!("{prefix}{msg} \n");
        }
        let dcall = deparse_call_line(cx, call);
        let line1 = msg.split('\n').next().unwrap_or("");
        let brk = if indent + wd(&dcall) + wd(line1) > LONGWARN { "\n " } else { "" };
        format!("{prefix}In {dcall} :{brk} {msg}\n")
    };
    if n == 1 {
        out.push_str("Warning message:\n");
        let (c, m) = ws[0];
        out.push_str(&item(c, &String::from_utf8_lossy(m.bytes()), "", 6));
    } else if n <= 10 {
        out.push_str("Warning messages:\n");
        for (i, &(c, m)) in ws.iter().enumerate() {
            out.push_str(&item(c, &String::from_utf8_lossy(m.bytes()), &format!("{}: ", i + 1), 10));
        }
    } else if n < robigo_rtcore::MAX_WARNINGS {
        out.push_str(&format!("There were {n} warnings (use warnings() to see them)\n"));
    } else {
        let m = robigo_rtcore::MAX_WARNINGS;
        out.push_str(&format!("There were {m} or more warnings (use warnings() to see the first {m})\n"));
    }
    drop(ws);
    cx.clear_warnings();
    io.err(out.as_bytes());
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

/// `deparse1s(call)`'s first line, for the calls this stand-in covers:
/// symbols, scalar constants without attributes, function-call syntax, and
/// the infix (`+`, `/`, `<-`, ...), prefix (`-`, `!`), `(`, `[` and `[[`
/// forms of those as GNU R's parser builds them (no precedence parentheses
/// added); a line ends where deparse breaks it (past [`CUTOFF`] bytes after
/// an argument's ", " or a binary operator). Anything else is a "not
/// implemented" text in its place (B3's deparse replaces this).
// R: deparse.c:deparse1s, deparse2buff, args2buff, linebreak
pub fn deparse_call_line(cx: &Cx, call: Ref<'_>) -> String {
    let mut s = String::new();
    match deparse(cx, call, &mut s) {
        Ok(()) | Err(Stop::LineFull) => s,
        Err(Stop::Unsupported) => "<robigo: deparse of this call is not implemented yet>".into(),
    }
}

/// Why [`deparse`] stopped early.
enum Stop {
    /// The first line is complete (deparse broke the line).
    LineFull,
    Unsupported,
}

/// `linebreak()`: the line ends here when it is past the cutoff.
fn linebreak(s: &str) -> Result<(), Stop> {
    if s.len() > CUTOFF { Err(Stop::LineFull) } else { Ok(()) }
}

fn deparse(cx: &Cx, x: Ref<'_>, s: &mut String) -> Result<(), Stop> {
    match x.sexptype() {
        SYMSXP => {
            sym_name(x, s, true);
            Ok(())
        }
        LANGSXP => deparse_call(cx, x, s),
        LGLSXP | INTSXP | REALSXP | STRSXP | NILSXP => constant(cx, x, s).ok_or(Stop::Unsupported),
        _ => Err(Stop::Unsupported),
    }
}

/// The text of a symbol, backquoted when it is not syntactic.
fn sym_name(x: Ref<'_>, s: &mut String, backtick: bool) {
    let n = String::from_utf8_lossy(x.as_sym().unwrap().name().bytes()).into_owned();
    if backtick && !is_valid_name(&n) {
        s.push('`');
        s.push_str(&n);
        s.push('`');
    } else {
        s.push_str(&n);
    }
}

/// `isValidName`.
// R: gram.y:isValidName
fn is_valid_name(n: &str) -> bool {
    const RESERVED: &[&str] = &[
        "NULL",
        "NA",
        "TRUE",
        "FALSE",
        "Inf",
        "NaN",
        "NA_integer_",
        "NA_real_",
        "NA_character_",
        "NA_complex_",
        "function",
        "while",
        "repeat",
        "for",
        "if",
        "in",
        "else",
        "next",
        "break",
    ];
    let b = n.as_bytes();
    let Some(&c) = b.first() else { return false };
    if c == b'.' {
        if b.get(1).is_some_and(u8::is_ascii_digit) {
            return false;
        }
    } else if !c.is_ascii_alphabetic() {
        return false;
    }
    if !b.iter().all(|&c| c.is_ascii_alphanumeric() || c == b'.' || c == b'_') {
        return false;
    }
    !RESERVED.contains(&n)
}

/// How names.c's PPinfo prints the operators this stand-in covers.
#[derive(Clone, Copy, PartialEq)]
enum Pp {
    /// PP_BINARY: ` op `, a line break after it.
    Binary,
    /// PP_BINARY2: no spaces.
    Binary2,
    /// PP_ASSIGN, PP_ASSIGN2: ` op `.
    Assign,
}

fn deparse_call(cx: &Cx, x: Ref<'_>, s: &mut String) -> Result<(), Stop> {
    let c = x.as_cons().ok_or(Stop::Unsupported)?;
    let f = c.car();
    let mut args = Vec::new();
    let mut a = c.cdr();
    while let Some(cell) = a.as_cons() {
        args.push((cell.tag(), cell.car()));
        a = cell.cdr();
    }
    if f.sexptype() != SYMSXP {
        return Err(Stop::Unsupported);
    }
    let name = String::from_utf8_lossy(f.as_sym().unwrap().name().bytes()).into_owned();
    let untagged = args.iter().all(|(t, _)| t.sexptype() == NILSXP);
    let pp = match name.as_str() {
        "+" | "-" | "*" | "==" | "!=" | "<" | ">" | "<=" | ">=" | "&" | "|" | "&&" | "||" => Some(Pp::Binary),
        "/" | "^" | "%%" | "%/%" | ":" => Some(Pp::Binary2),
        "<-" | "<<-" | "=" => Some(Pp::Assign),
        _ => None,
    };
    if untagged {
        match (pp, args.len()) {
            (Some(pp), 2) => {
                deparse(cx, args[0].1, s)?;
                if pp == Pp::Binary2 {
                    s.push_str(&name);
                } else {
                    s.push(' ');
                    s.push_str(&name);
                    s.push(' ');
                }
                if pp == Pp::Binary {
                    linebreak(s)?;
                }
                return deparse(cx, args[1].1, s);
            }
            (Some(Pp::Binary), 1) if matches!(name.as_str(), "-" | "+") => {
                s.push_str(&name);
                return deparse(cx, args[0].1, s);
            }
            _ => {}
        }
        if name == "!" && args.len() == 1 {
            s.push('!');
            return deparse(cx, args[0].1, s);
        }
        if name == "(" && args.len() == 1 {
            s.push('(');
            deparse(cx, args[0].1, s)?;
            s.push(')');
            return Ok(());
        }
    }
    if matches!(name.as_str(), "[" | "[[") && !args.is_empty() && args[0].0.sexptype() == NILSXP {
        deparse(cx, args[0].1, s)?;
        s.push_str(&name);
        arg_list(cx, &args[1..], s)?;
        s.push_str(if name == "[" { "]" } else { "]]" });
        return Ok(());
    }
    if pp.is_some()
        || (name.starts_with('%') && name.ends_with('%') && name.len() >= 2)
        || matches!(
            name.as_str(),
            "{" | "("
                | "!"
                | "if"
                | "for"
                | "while"
                | "repeat"
                | "function"
                | "$"
                | "@"
                | "::"
                | ":::"
                | "~"
                | "?"
                | "break"
                | "next"
                | "["
                | "[["
        )
    {
        return Err(Stop::Unsupported);
    }
    sym_name(f, s, true);
    s.push('(');
    arg_list(cx, &args, s)?;
    s.push(')');
    Ok(())
}

/// `args2buff` (not formals).
fn arg_list(cx: &Cx, args: &[(Ref<'_>, Ref<'_>)], s: &mut String) -> Result<(), Stop> {
    let missing = cx.missing_arg().sexp();
    for (i, &(tag, v)) in args.iter().enumerate() {
        if tag.sexptype() == SYMSXP {
            sym_name(tag, s, true);
            s.push_str(" = ");
        }
        if v.sexp() != missing {
            deparse(cx, v, s)?;
        }
        if i + 1 < args.len() {
            s.push_str(", ");
            linebreak(s)?;
        }
    }
    Ok(())
}

/// A scalar constant as deparse writes it (`vec2buff` with one element and
/// the default options: `keepInteger`, `keepNA`, 15 significant digits).
fn constant(cx: &Cx, x: Ref<'_>, s: &mut String) -> Option<()> {
    if x.sexptype() == NILSXP {
        s.push_str("NULL");
        return Some(());
    }
    if x.attrib().is_some_and(|a| a.sexptype() != NILSXP) || x.len()? != 1 {
        return None;
    }
    let pp = robigo_text::PrintParams { digits: 15, ..robigo_text::PrintParams::DEFAULT };
    match x.sexptype() {
        LGLSXP => {
            let v = x.as_vector::<Lgl>()?.get(0);
            s.push_str(if v == Lgl::NA {
                "NA"
            } else if v.0 != 0 {
                "TRUE"
            } else {
                "FALSE"
            });
        }
        INTSXP => {
            let v = x.as_vector::<i32>()?.get(0);
            if v == i32::MIN {
                s.push_str("NA_integer_");
            } else {
                s.push_str(&format!("{v}L"));
            }
        }
        REALSXP => {
            let v = x.as_vector::<f64>()?.get(0);
            if robigo_text::is_na(v) {
                s.push_str("NA_real_");
            } else {
                let f = robigo_text::num::format_real(&[v], 0, &pp);
                let mut b = Vec::new();
                robigo_text::num::encode_real0(&mut b, v, robigo_text::num::RealFormat { w: 0, ..f }, b".", &pp);
                s.push_str(&String::from_utf8_lossy(&b));
            }
        }
        STRSXP => {
            let c = x.as_vector::<robigo_rtcore::Char<'_>>()?.get(0);
            if c == cx.na_string() {
                s.push_str("NA_character_");
            } else {
                let mut b = Vec::new();
                robigo_text::string::encode_string(
                    &mut b,
                    Some(robigo_rtbuiltins::coerce::text(c)),
                    0,
                    b'"',
                    robigo_text::string::Justify::Left,
                    false,
                    &pp,
                );
                s.push_str(&String::from_utf8_lossy(&b));
            }
        }
        _ => return None,
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_names() {
        for n in ["a", "a.b", ".a", "a_1", "..x", "TRUE_", "...", "..1"] {
            assert!(is_valid_name(n), "{n}");
        }
        for n in ["", "_a", ".1", "1a", "if", "TRUE", "a b", "+"] {
            assert!(!is_valid_name(n), "{n}");
        }
    }
}
