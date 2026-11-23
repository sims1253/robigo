//! The reader: R source text to language objects, as GNU R's
//! `R_ParseVector` gives them to C callers (A1's `robigo-rtapi`), over
//! `robigo-syntax`'s parser and [`crate::lower`]. No source references:
//! robigo-rt is Rscript, where `keep.source` is FALSE, and `R_ParseVector`'s
//! callers pass `srcfile = R_NilValue`.
//!
//! The console's statement-at-a-time parse (`R_Parse1Buffer`) is
//! [`crate::repl`]'s; the base loader parses whole files ([`crate::base`]).

use robigo_rtcore::{Char, Cx, Out, RResult, Ref, Root};

/// GNU R's `ParseStatus` (R_ext/Parse.h), with its values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum ParseStatus {
    Null = 0,
    Ok = 1,
    Incomplete = 2,
    Error = 3,
    Eof = 4,
}

/// What GNU R leaves in `R_ParseError`, `R_ParseErrorCol` and
/// `R_ParseErrorMsg` after a failed parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseFailure {
    /// `R_ParseError`: the line of the error (1-based).
    pub line: usize,
    /// `R_ParseErrorCol`.
    pub col: usize,
    /// `R_ParseErrorMsg`: the message without position or context.
    pub message: String,
}

/// `R_ParseVector(text, n, &status, R_NilValue)`: parse the lines of the
/// character vector `text` (each element one line, `translateChar`ed) and
/// write to `out` an expression vector of the first `n` expressions (all of
/// them when `n < 0`), or `R_NilValue` when the status is not
/// [`ParseStatus::Ok`]. The lexer's warnings are signalled as GNU R's
/// parser signals them (`warning()`), before the result is known.
///
/// `text` must be a character vector (GNU R's callers check; this panics
/// on anything else).
// R: source.c:R_ParseVector, gram.y:R_Parse, R_TextBufferInit
pub fn parse_vector(cx: &mut Cx, text: Root<'_>, n: i32, out: Out<'_>) -> RResult<(ParseStatus, Option<ParseFailure>)> {
    // R_TextBufferInit: every element translated, followed by a newline
    let mut src = Vec::new();
    {
        let loc = cx.locale();
        let v = text.bind(cx).as_vector::<Char<'_>>().expect("robigo: R_ParseVector of a non-character vector");
        for i in 0..v.len() {
            let s = robigo_rtbuiltins::coerce::text(v.get(i));
            match robigo_rtbuiltins::text::to_native(s, loc) {
                Ok(b) => src.extend_from_slice(&b),
                // R: sysutils.c:translateChar
                Err(_) => return Err(cx.error("translating strings with \"bytes\" encoding is not allowed")),
            }
            src.push(b'\n');
        }
    }
    parse_bytes(cx, &src, n, out)
}

/// [`parse_vector`] of source bytes in the native (UTF-8) encoding.
///
/// The lexer's warnings are those of the tokens GNU R's parse reads: R_Parse
/// lexes on demand, expression by expression, so it stops after the `n`-th
/// expression's terminator, and a syntax error stops it at the token it
/// rejects (the warnings before it are still signalled).
// R: gram.y:R_Parse, R_Parse1
pub fn parse_bytes(cx: &mut Cx, src: &[u8], n: i32, out: Out<'_>) -> RResult<(ParseStatus, Option<ParseFailure>)> {
    let text = robigo_syntax::lexable(src);
    // known_to_be_utf8: the session's (mkString2's UTF-8 mark)
    let source = robigo_syntax::Source::new(&text, src).known_to_be_utf8(cx.locale().utf8);
    let arena = robigo_syntax::Arena::new();
    // every token's warnings with their positions (the lexer never fails:
    // a lexical error is a token the parser rejects)
    let warnings = match robigo_syntax::lexer::Lexer::new(source).lex() {
        Ok(l) => l.warnings,
        Err(_) => Vec::new(),
    };
    let signal = |cx: &mut Cx, upto: usize| -> RResult<()> {
        for w in warnings.iter().filter(|w| (w.at as usize) <= upto) {
            cx.warning(&w.message)?;
        }
        Ok(())
    };
    let exprs = match robigo_syntax::parse_program(&arena, source) {
        Ok(r) => r,
        Err(e) => {
            if n >= 0 {
                // R_Parse stops after the n-th expression, so an error
                // after it is no error there; this parse does not stop.
                return Err(
                    cx.error("robigo: R_ParseVector with n >= 0 on text with a parse error is not implemented yet")
                );
            }
            check_non_ascii_locale(cx, src)?;
            // raiseParseError/raiseLexError signal from inside the parse
            if e.style == robigo_syntax::ErrorStyle::Condition {
                signal(cx, e.offset)?;
                return Err(crate::parse_cond::signal(cx, source, &e));
            }
            let (line, col) = robigo_syntax::error_line_col(source, &e);
            let status = if e.incomplete { ParseStatus::Incomplete } else { ParseStatus::Error };
            signal(cx, if e.incomplete { usize::MAX } else { e.offset })?;
            out.set(cx, cx.nil());
            return Ok((status, Some(ParseFailure { line, col, message: e.message })));
        }
    };
    let take = if n < 0 { exprs.len() } else { exprs.len().min(n as usize) };
    check_non_ascii_locale(cx, src)?;
    // the tokens up to the next expression's start (its terminator and
    // what separates them carry no warnings)
    signal(cx, if take < exprs.len() { (exprs[take].span().start as usize).saturating_sub(1) } else { usize::MAX })?;
    lower_all(cx, &exprs[..take], cx.locale().utf8, out)?;
    Ok((ParseStatus::Ok, None))
}

/// Lower `exprs`, of a parse whose `known_to_be_utf8` is `known`, into a
/// new expression vector written to `out`.
pub fn lower_all(cx: &mut Cx, exprs: &[robigo_syntax::Expr<'_>], known: bool, out: Out<'_>) -> RResult<()> {
    cx.build(out, |cx| {
        let mut v = cx.alloc_expression(exprs.len())?;
        for (i, e) in exprs.iter().enumerate() {
            let x = crate::lower::lower_known(cx, e, known)?;
            // SAFETY: fresh, reachable from the expression vector from here;
            // nothing collects under `&Cx`.
            #[allow(unsafe_code)]
            v.set(i, unsafe { Ref::from_sexp(x, cx) });
        }
        Ok(v.finish().into())
    })
}

/// One expression lowered into `out` (the console's statement).
pub fn lower_one(cx: &mut Cx, e: &robigo_syntax::Expr<'_>, out: Out<'_>) -> RResult<()> {
    cx.build(out, |cx| {
        let x = crate::lower::lower(cx, e)?;
        // SAFETY: fresh; nothing collects under `&Cx`.
        #[allow(unsafe_code)]
        Ok(unsafe { Ref::from_sexp(x, cx) })
    })
}

/// In a locale that is not UTF-8, GNU R's lexer reads bytes with the C
/// library's single-byte classes (`isalpha`, `isspace`: a non-ASCII byte
/// starts no name and is no white space), where robigo-syntax's classifies
/// Unicode characters. So a non-ASCII byte outside a string literal, a
/// backquoted name and a comment is refused, anywhere in the input: R_Parse
/// may stop after the requested expressions, but where its lexer stops is
/// past white space the expressions' spans do not show, so all of it is
/// checked (refusing more than GNU R rejects, never accepting what it does).
// R: gram.y:token, SkipSpace, SymbolValue (mbcslocale FALSE)
pub(crate) fn check_non_ascii_locale(cx: &mut Cx, src: &[u8]) -> RResult<()> {
    if non_ascii_refused(cx, src) {
        return Err(cx.error(NON_ASCII_REFUSED));
    }
    Ok(())
}

/// [`check_non_ascii_locale`]'s refusal.
pub(crate) const NON_ASCII_REFUSED: &str =
    "robigo: non-ASCII characters outside strings and comments in a non-UTF-8 locale are not implemented yet";

/// Whether [`check_non_ascii_locale`] refuses `src`.
pub(crate) fn non_ascii_refused(cx: &Cx, src: &[u8]) -> bool {
    let upto = src.len();
    if cx.locale().utf8 || src.is_ascii() {
        return false;
    }
    let text = robigo_syntax::lexable(src);
    let source = robigo_syntax::Source::new(&text, src).known_to_be_utf8(false);
    let Ok(lexed) = robigo_syntax::lexer::Lexer::new(source).lex() else { return true };
    let mut toks = lexed.toks.iter().map(|t| (t.span.start as usize, t.span.end as usize, &t.tok)).peekable();
    let mut i = 0;
    while i < upto {
        while toks.peek().is_some_and(|&(_, e, _)| e <= i) {
            toks.next();
        }
        if let Some(&(st, e, tok)) = toks.peek()
            && st <= i
        {
            // a string's or a backquoted name's bytes are read as GNU R reads
            // them; any other token with a non-ASCII byte is not
            let quoted = matches!(tok, robigo_syntax::lexer::Tok::Str(_))
                || (matches!(tok, robigo_syntax::lexer::Tok::Sym(_)) && src.get(st) == Some(&b'`'));
            if !quoted && !src[i..e.min(upto)].is_ascii() {
                return true;
            }
            i = e;
            continue;
        }
        match src[i] {
            // a comment, to the end of the line
            b'#' => {
                while i < upto && src[i] != b'\n' {
                    i += 1;
                }
            }
            b if b >= 0x80 => return true,
            _ => i += 1,
        }
    }
    false
}
