//! Top-level printing, the part M1's first cases need: `PrintValueEnv` for
//! values that are not objects or functions, and `PrintValueRec` for NULL
//! and the logical, integer, double and character vectors without
//! attributes (print.c, printvector.c), over `robigo-text`'s ports of
//! format.c and printutils.c. Everything else, print()'s dispatch and
//! show() included, raises "not implemented yet" (B3's rt glue, S1).
//!
//! The settings are `PrintDefaults()` with `options()` at their defaults
//! (`digits` 7, `width` 80, `max.print` 99999, `OutDec` "."), which is all
//! robigo-rt has until `options()` exists (B2b).

use robigo_heap::types::*;
use robigo_rtcore::{Char, Cx, Lgl, RResult, Ref, Root};
use robigo_text::num::{
    encode_integer, encode_logical, encode_real0, format_integer, format_logical, format_real, index_width,
};
use robigo_text::string::{Justify, encode_string, format_string};
use robigo_text::{PrintParams, Str};

/// `R_print.width`: `options(width)`'s default.
const WIDTH: i32 = 80;
/// `R_print.gap`.
const GAP: i32 = 1;
/// `R_print.max`: `options(max.print)`'s default.
const MAX: usize = 99999;

/// `PrintValueEnv(x, env)` as the top level calls it for a visible value:
/// the text appended to `out`.
// R: print.c:PrintValueEnv
pub fn print_value_env(cx: &mut Cx, x: Root<'_>, out: &mut Vec<u8>) -> RResult<()> {
    let v = x.bind(cx);
    if v.is_object() || matches!(v.sexptype(), CLOSXP | BUILTINSXP | SPECIALSXP) {
        // PrintObject: print(x) or show(x) through the evaluator
        return Err(cx.error("robigo: printing an object or a function (print, show) is not implemented yet"));
    }
    print_value_rec(cx, x, out)
}

/// `PrintValueRec` for the types this module prints.
// R: print.c:PrintValueRec
fn print_value_rec(cx: &mut Cx, x: Root<'_>, out: &mut Vec<u8>) -> RResult<()> {
    let v = x.bind(cx);
    let t = v.sexptype();
    if t == NILSXP {
        out.extend_from_slice(b"NULL\n");
        return Ok(());
    }
    if !matches!(t, LGLSXP | INTSXP | REALSXP | STRSXP) {
        let msg = format!("robigo: printing a value of type {} is not implemented yet", type_name(t));
        return Err(cx.error(&msg));
    }
    if v.attrib().is_some_and(|a| a.sexptype() != NILSXP) {
        return Err(cx.error("robigo: printing a vector with attributes is not implemented yet"));
    }
    let pp = PrintParams { locale: robigo_rtbuiltins::text::ctype(cx.locale()), ..PrintParams::DEFAULT };
    print_vector(v, cx.na_string(), &pp, out);
    Ok(())
}

/// `printVector(x, indx = 1, quote = 1)`.
// R: printvector.c:printVector
fn print_vector<'g>(v: Ref<'g>, na: Char<'g>, pp: &PrintParams, out: &mut Vec<u8>) {
    let n = v.len().expect("robigo: a vector");
    if n == 0 {
        out.extend_from_slice(match v.sexptype() {
            LGLSXP => b"logical(0)\n" as &[u8],
            INTSXP => b"integer(0)\n",
            REALSXP => b"numeric(0)\n",
            _ => b"character(0)\n",
        });
        return;
    }
    let n_pr = if n <= MAX + 1 { n } else { MAX };
    match v.sexptype() {
        LGLSXP => {
            let x: Vec<i32> = {
                let a = v.as_vector::<Lgl>().unwrap();
                (0..n_pr).map(|i| a.get(i).0).collect()
            };
            let w = format_logical(&x, pp) + GAP;
            num_loop(n_pr, w, out, |o, i| encode_logical(o, x[i], w, pp));
        }
        INTSXP => {
            let x: Vec<i32> = {
                let a = v.as_vector::<i32>().unwrap();
                (0..n_pr).map(|i| a.get(i)).collect()
            };
            let w = format_integer(&x, pp) + GAP;
            num_loop(n_pr, w, out, |o, i| encode_integer(o, x[i], w, pp));
        }
        REALSXP => {
            let x: Vec<f64> = {
                let a = v.as_vector::<f64>().unwrap();
                (0..n_pr).map(|i| a.get(i)).collect()
            };
            let mut f = format_real(&x, 0, pp);
            f.w += GAP;
            num_loop(n_pr, f.w, out, |o, i| encode_real0(o, x[i], f, b".", pp));
        }
        _ => {
            let a = v.as_vector::<Char<'_>>().unwrap();
            let s = |i: usize| -> Option<Str<'_>> {
                let c = a.get(i);
                if c == na { None } else { Some(robigo_rtbuiltins::coerce::text(c)) }
            };
            let quote = b'"';
            let w = format_string((0..n_pr).map(s), quote, pp);
            // CHARVECTOR_TIGHTLOOP
            let labwidth = index_width(n_pr as i64) + 2;
            vector_index(out, 1, labwidth);
            let mut width = labwidth;
            for i in 0..n_pr {
                if i > 0 && width + w + GAP > WIDTH {
                    out.push(b'\n');
                    vector_index(out, i + 1, labwidth);
                    width = labwidth;
                }
                out.extend(std::iter::repeat_n(b' ', GAP as usize));
                encode_string(out, s(i), w, quote, Justify::Left, false, pp);
                width += w + GAP;
            }
            out.push(b'\n');
        }
    }
    if n_pr < n {
        out.extend_from_slice(
            format!(" [ reached 'max' / getOption(\"max.print\") -- omitted {} entries ]\n", n - n_pr).as_bytes(),
        );
    }
}

/// NUMVECTOR_TIGHTLOOP with DO_first_lab and DO_newline (`indx` 1).
fn num_loop(n: usize, w: i32, out: &mut Vec<u8>, mut enc: impl FnMut(&mut Vec<u8>, usize)) {
    let labwidth = index_width(n as i64) + 2;
    vector_index(out, 1, labwidth);
    let mut width = labwidth;
    for i in 0..n {
        if i > 0 && width + w > WIDTH {
            out.push(b'\n');
            vector_index(out, i + 1, labwidth);
            width = labwidth;
        }
        enc(out, i);
        width += w;
    }
    out.push(b'\n');
}

/// `VectorIndex(i, w)`: `[i]` right-justified in `w`.
// R: printutils.c:VectorIndex
fn vector_index(out: &mut Vec<u8>, i: usize, w: i32) {
    let pad = (w - index_width(i as i64) - 2).max(0) as usize;
    out.extend(std::iter::repeat_n(b' ', pad));
    out.extend_from_slice(format!("[{i}]").as_bytes());
}

/// `type2char`.
fn type_name(t: u8) -> &'static str {
    match t {
        NILSXP => "NULL",
        SYMSXP => "symbol",
        LISTSXP => "pairlist",
        CLOSXP => "closure",
        ENVSXP => "environment",
        PROMSXP => "promise",
        LANGSXP => "language",
        SPECIALSXP => "special",
        BUILTINSXP => "builtin",
        CHARSXP => "char",
        LGLSXP => "logical",
        INTSXP => "integer",
        REALSXP => "double",
        CPLXSXP => "complex",
        STRSXP => "character",
        DOTSXP => "...",
        ANYSXP => "any",
        EXPRSXP => "expression",
        VECSXP => "list",
        EXTPTRSXP => "externalptr",
        BCODESXP => "bytecode",
        WEAKREFSXP => "weakref",
        RAWSXP => "raw",
        OBJSXP => "object",
        _ => "unknown",
    }
}
