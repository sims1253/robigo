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
pub fn parse_bytes(cx: &mut Cx, src: &[u8], n: i32, out: Out<'_>) -> RResult<(ParseStatus, Option<ParseFailure>)> {
    let text = robigo_syntax::lexable(src);
    let source = robigo_syntax::Source::new(&text, src);
    let arena = robigo_syntax::Arena::new();
    let (exprs, warnings) = match robigo_syntax::parse_program_with_warnings(&arena, source) {
        Ok(r) => r,
        Err(e) => {
            if n >= 0 {
                // R_Parse stops after the n-th expression, so an error
                // after it is no error there; this parse does not stop.
                return Err(
                    cx.error("robigo: R_ParseVector with n >= 0 on text with a parse error is not implemented yet")
                );
            }
            let (line, col) = robigo_syntax::error_line_col(source, &e);
            let status = if e.incomplete { ParseStatus::Incomplete } else { ParseStatus::Error };
            out.set(cx, cx.nil());
            return Ok((status, Some(ParseFailure { line, col, message: e.message })));
        }
    };
    for w in &warnings {
        cx.warning(w)?;
    }
    let take = if n < 0 { exprs.len() } else { exprs.len().min(n as usize) };
    lower_all(cx, &exprs[..take], out)?;
    Ok((ParseStatus::Ok, None))
}

/// Lower `exprs` into a new expression vector written to `out`.
pub fn lower_all(cx: &mut Cx, exprs: &[robigo_syntax::Expr<'_>], out: Out<'_>) -> RResult<()> {
    cx.build(out, |cx| {
        let mut v = cx.alloc_expression(exprs.len())?;
        for (i, e) in exprs.iter().enumerate() {
            let x = crate::lower::lower(cx, e)?;
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
