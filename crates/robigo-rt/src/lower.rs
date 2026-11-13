//! R source to language objects: `robigo-syntax`'s parse tree lowered to
//! the heap's calls, symbols and constants, with no srcrefs (Rscript's
//! `keep.source = FALSE`). The parser already shapes its tree as GNU R's
//! `parse()` does, so the lowering makes no decisions of its own.
//!
//! L1 owns the reader and `R_ParseVector`; this module is the lowering they
//! build on, here first because the evaluator's tests need source text.

#![allow(unsafe_code)]

use robigo_heap::types::*;
use robigo_heap::{NeedGc, Sexp};
use robigo_rtcore::{Cx, Encoding, Out, RResult, Ref};
use robigo_syntax::ast::{Const, Expr};

/// Parse `src` and write its expressions as an EXPRSXP to `out`. A syntax
/// error is reported with the parser's message (L1 owns GNU R's format).
pub fn parse(cx: &mut Cx, src: &str, out: Out<'_>) -> RResult<()> {
    let arena = robigo_syntax::Arena::new();
    let exprs = match robigo_syntax::parse_program(&arena, src) {
        Ok(e) => e,
        Err(e) => return Err(cx.error(&e.message)),
    };
    cx.build(out, |cx| {
        let mut v = cx.alloc_expression(exprs.len())?;
        for (i, e) in exprs.iter().enumerate() {
            let x = lower(cx, e)?;
            // SAFETY: fresh, reachable from the expression vector from here.
            v.set(i, unsafe { Ref::from_sexp(x, cx) });
        }
        Ok(v.finish().into())
    })
}

/// One expression as a language object, under `&Cx` (never collects; the
/// caller retries the whole lowering after a refusal).
pub fn lower(cx: &Cx, e: &Expr<'_>) -> Result<Sexp, NeedGc> {
    let nil = cx.nil().sexp();
    Ok(match *e {
        Expr::Const(c, _) => constant(cx, c)?,
        Expr::Sym(name, _) => cx.install(name.as_bytes())?.as_ref().sexp(),
        Expr::Placeholder(_) => cx.install(b"_")?.as_ref().sexp(),
        Expr::Call { fun, args, .. } => {
            // build the argument list from the end
            let mut list = nil;
            for a in args.iter().rev() {
                let v = match &a.value {
                    Some(v) => lower(cx, v)?,
                    None => cx.missing_arg().sexp(),
                };
                let tag = match a.name {
                    Some(n) => cx.install(n.as_bytes())?.as_ref().sexp(),
                    None => nil,
                };
                // SAFETY: fresh objects and immortal symbols of this heap;
                // nothing collects under `&Cx`.
                list = unsafe { cx.alloc_node_raw(LISTSXP, v, list, tag)? };
            }
            let f = lower(cx, fun)?;
            // SAFETY: as above.
            unsafe { cx.alloc_node_raw(LANGSXP, f, list, nil)? }
        }
        Expr::Function { formals, body, .. } => {
            // `function`(formals, body, srcref = NULL), as gram.y's xxdefun
            // makes it without keep.source
            let mut fl = nil;
            for f in formals.iter().rev() {
                let v = match &f.default {
                    Some(d) => lower(cx, d)?,
                    None => cx.missing_arg().sexp(),
                };
                let tag = cx.install(f.name.as_bytes())?.as_ref().sexp();
                // SAFETY: as above.
                fl = unsafe { cx.alloc_node_raw(LISTSXP, v, fl, tag)? };
            }
            let b = lower(cx, body)?;
            let function = cx.install(b"function")?.as_ref().sexp();
            // SAFETY: as above.
            unsafe {
                let l = cx.alloc_node_raw(LISTSXP, nil, nil, nil)?;
                let l = cx.alloc_node_raw(LISTSXP, b, l, nil)?;
                let l = cx.alloc_node_raw(LISTSXP, fl, l, nil)?;
                cx.alloc_node_raw(LANGSXP, function, l, nil)?
            }
        }
    })
}

/// A parsed constant.
fn constant(cx: &Cx, c: Const<'_>) -> Result<Sexp, NeedGc> {
    const NA_INT: i32 = i32::MIN;
    Ok(match c {
        Const::Null => cx.nil().sexp(),
        Const::Lgl(b) => {
            let v = b.map_or(NA_INT, i32::from);
            cx.alloc_with(1, |_| robigo_rtcore::Lgl(v))?.finish().as_ref().sexp()
        }
        Const::Int(i) => {
            let v = i.unwrap_or(NA_INT);
            cx.alloc_with(1, |_| v)?.finish().as_ref().sexp()
        }
        Const::Dbl(x) => cx.alloc_with(1, |_| x)?.finish().as_ref().sexp(),
        Const::DblNa => cx.alloc_with(1, |_| robigo_numerics::NA_REAL)?.finish().as_ref().sexp(),
        Const::Cplx(r, i) => cx.alloc_with(1, |_| robigo_heap::Rcomplex { r, i })?.finish().as_ref().sexp(),
        Const::CplxNa => cx
            .alloc_with(1, |_| robigo_heap::Rcomplex { r: robigo_numerics::NA_REAL, i: robigo_numerics::NA_REAL })?
            .finish()
            .as_ref()
            .sexp(),
        Const::Str(s) => string(cx, s, None)?,
        Const::StrNative(b) => string(cx, Some(b), Some(Encoding::Native))?,
        Const::StrUtf8(b) => string(cx, Some(b), Some(Encoding::Utf8))?,
    })
}

/// A character vector of one string (NA for None). A string with no
/// declared encoding is native when ASCII and UTF-8 otherwise (a UTF-8
/// session, `known_to_be_utf8`).
fn string(cx: &Cx, b: Option<&[u8]>, enc: Option<Encoding>) -> Result<Sexp, NeedGc> {
    let c = match b {
        None => cx.na_string(),
        Some(b) => {
            let enc = enc.unwrap_or(if b.is_ascii() { Encoding::Native } else { Encoding::Utf8 });
            cx.mk_char(b, enc)?
        }
    };
    let mut v = cx.alloc_vector::<robigo_rtcore::Char<'_>>(1)?;
    v.set(0, c);
    Ok(v.finish().as_ref().sexp())
}
