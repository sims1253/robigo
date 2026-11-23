//! gram.y's classed parse errors: `raiseParseError` (a parser action's
//! check: the native pipe and its placeholder, a repeated formal, a tag of
//! the wrong type) and `raiseLexError` (the lexer's: escapes, multibyte
//! characters) make a `parseError` condition and signal it from inside the
//! parser, whoever called it: `parse()` and `source()`, C code's
//! `R_ParseVector`, the console, a profile. [`signal`] is that one
//! condition, raised through the classed-error hook as `R_signalErrorCondition`
//! raises it; every parse path calls it for an error of
//! [`robigo_syntax::ErrorStyle::Condition`].

#![allow(unsafe_code)]

use robigo_heap::Sexp;
use robigo_heap::types::*;
use robigo_rtcore::{Cx, NeedGc, RError, Ref};
use robigo_syntax::{ConditionValue, ErrorExpr, ParseError, Source};

/// The condition's `filename` without a source file: gram.y's
/// `getFilename()` reads the `filename` of the parse's srcfile environment
/// and gives "<input>" without one. robigo-rt's parses keep no source
/// references yet (`keep.source = TRUE` is refused), so this is every
/// parse's; the srcfile's name is E's seam (source references).
// R: gram.y:getFilename (gram.y:4426)
const NO_SRCFILE: &str = "<input>";

/// `raiseParseError(subclass, call, valuetype, value, lloc, format)` /
/// `raiseLexError(...)` for the parse error `e` of `src`: the condition of
/// class `c(e.subclass, "parseError", "error", "condition")`, its message
/// ending in `(<filename>:<lineno>:<colno>)`, with the fields `value`,
/// `filename`, `lineno` and `colno` (gram.y's line and column: 1-based
/// lines, columns counted in characters with tabs to the next multiple of
/// 8), its call the expression the parser passed (`NULL` for the lexer's),
/// signalled: the handlers for its classes, then the default handling.
// R: gram.y:raiseParseError, raiseLexError; errors.c:R_makeErrorCondition, R_signalErrorCondition
pub fn signal(cx: &mut Cx, src: Source<'_>, e: &ParseError) -> RError {
    debug_assert!(e.style == robigo_syntax::ErrorStyle::Condition, "robigo: a parse error that is no condition");
    let (line, col) = robigo_syntax::error_line_col(src, e);
    let msg = robigo_syntax::condition_message(src, NO_SRCFILE, e);
    let built = cx.scope(|cx, s| -> Result<RError, RError> {
        let value = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                Ok(match &e.value {
                    ConditionValue::Null => cx.nil(),
                    ConditionValue::Int(v) => cx.alloc_with(1, |_| *v)?.finish().as_ref(),
                    ConditionValue::Str(v) => cx.mk_string(v.as_bytes(), robigo_rtcore::Encoding::Native)?,
                })
            })
        })?;
        let filename = s.capture(cx, |cx, o| {
            cx.build(o, |cx| cx.mk_string(NO_SRCFILE.as_bytes(), robigo_rtcore::Encoding::Native))
        })?;
        let lineno =
            s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.alloc_with(1, |_| line as i32)?.finish().as_ref())))?;
        let colno = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.alloc_with(1, |_| col as i32)?.finish().as_ref())))?;
        let call = match e.call.as_deref() {
            None => None,
            Some(c) => Some(s.capture(cx, |cx, o| {
                // SAFETY: a fresh object of this heap, the result of the
                // build (nothing collects under `&Cx`).
                cx.build(o, |cx| Ok(unsafe { Ref::from_sexp(error_expr(cx, c)?, cx) }))
            })?),
        };
        let fields = [("value", value), ("filename", filename), ("lineno", lineno), ("colno", colno)];
        let cond = robigo_rtcore::ErrorCond {
            class: "parseError",
            subclass: Some(e.subclass),
            msg: msg.as_bytes(),
            fields: &fields,
            exit_only: false,
        };
        Ok(cx.error_condition_call(call, &cond))
    });
    match built {
        Ok(e) | Err(e) => e,
    }
}

/// The parser's offending expression as the language object gram.y holds:
/// a call with its arguments (an empty one R_MissingArg), `function`'s
/// `function(formals, body, NULL)`, a symbol, a constant.
// R: gram.y:xxpipe, xxdefun (the objects raiseParseError is given)
fn error_expr(cx: &robigo_rtcore::Cx, e: &ErrorExpr) -> Result<Sexp, NeedGc> {
    let nil = cx.nil().sexp();
    Ok(match e {
        ErrorExpr::Atom(c) => crate::lower::constant(cx, *c, cx.locale().utf8)?,
        ErrorExpr::String(s) => crate::lower::constant(cx, robigo_syntax::Const::Str(s.as_deref()), cx.locale().utf8)?,
        ErrorExpr::Symbol(s) => cx.install(s.as_bytes())?.as_ref().sexp(),
        ErrorExpr::Call(fun, args) => {
            let mut list = nil;
            for (name, value) in args.iter().rev() {
                let v = match value {
                    Some(v) => error_expr(cx, v)?,
                    None => cx.missing_arg().sexp(),
                };
                let tag = match name {
                    Some(n) => cx.install(n.as_bytes())?.as_ref().sexp(),
                    None => nil,
                };
                // SAFETY: fresh objects and immortal symbols of this heap;
                // nothing collects under `&Cx`.
                list = unsafe { cx.alloc_node_raw(LISTSXP, v, list, tag)? };
            }
            let f = error_expr(cx, fun)?;
            // SAFETY: as above.
            unsafe { cx.alloc_node_raw(LANGSXP, f, list, nil)? }
        }
        ErrorExpr::Function(formals, body) => {
            let mut fl = nil;
            for (name, default) in formals.iter().rev() {
                let v = match default {
                    Some(d) => error_expr(cx, d)?,
                    None => cx.missing_arg().sexp(),
                };
                let tag = cx.install(name.as_bytes())?.as_ref().sexp();
                // SAFETY: as above.
                fl = unsafe { cx.alloc_node_raw(LISTSXP, v, fl, tag)? };
            }
            let b = error_expr(cx, body)?;
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
