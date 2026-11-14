//! array.c: `.Internal(matrix(data, nrow, ncol, byrow, dimnames,
//! missing(nrow), missing(ncol)))` (`do_matrix`, with duplicate.c's
//! `copyMatrix`). `gc()`'s own R code shapes its result with it.

use crate::attrib::set::dimnamesgets;
use crate::attrib::{Attr, Attrs};
use crate::coerce::vector::as_integer_warn;
use crate::util::{as_logical, type_to_char};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Lgl, Out, RResult, Rcomplex, Ref, Root, View};

const NA_INTEGER: i32 = i32::MIN;
const NA_REAL: f64 = crate::kernel::arith::NA_REAL;
/// `R_XLEN_T_MAX`.
const R_XLEN_T_MAX: f64 = 4503599627370496.0;

/// GNU R's `isNumeric`: an integer (not a factor), logical or double vector.
fn is_numeric(x: Ref<'_>) -> bool {
    match x.sexptype() {
        INTSXP => !crate::attrib::inherits(x, b"factor"),
        LGLSXP | REALSXP => true,
        _ => false,
    }
}

/// `.Internal(matrix(...))`.
// R: array.c:do_matrix
pub fn do_matrix(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("matrix", true).unwrap().check_arity(cx, call, args)?;
    let vals = args.root(0);
    let v = vals.bind(cx);
    if !crate::attrib::is_vector(v) {
        let msg = format!("'data' must be of a vector type, was '{}'", type_to_char(v));
        return Err(cx.error(&msg));
    }
    if v.is_package_altrep() {
        return Err(crate::util::not_yet(cx, Some(call), "matrix() of an ALTREP vector"));
    }
    let lendat = v.len().unwrap_or(0);
    let byrow = as_logical(cx, args.get(cx, 3));
    if byrow == NA_INTEGER {
        return Err(cx.error("invalid 'byrow' argument"));
    }
    let byrow = byrow != 0;
    let miss_nr = as_logical(cx, args.get(cx, 5)) != 0;
    let miss_nc = as_logical(cx, args.get(cx, 6)) != 0;
    let (mut nr, mut nc) = (1i64, 1i64);
    for (missing, k, what) in [(miss_nr, 1, "nrow"), (miss_nc, 2, "ncol")] {
        if missing {
            continue;
        }
        if !is_numeric(args.get(cx, k)) {
            return Err(cx.error("non-numeric matrix extent"));
        }
        let n = as_integer_warn(cx, args.root(k))?;
        if n == NA_INTEGER {
            return Err(cx.error(&format!("invalid '{what}' value (too large or NA)")));
        }
        if n < 0 {
            return Err(cx.error(&format!("invalid '{what}' value (< 0)")));
        }
        if k == 1 { nr = n as i64 } else { nc = n as i64 }
    }
    let lendat_f = lendat as f64;
    if miss_nr && miss_nc {
        if lendat > i32::MAX as usize {
            return Err(cx.error("data is too long"));
        }
        nr = lendat as i64;
    } else if miss_nr {
        if lendat_f > nc as f64 * i32::MAX as f64 {
            return Err(cx.error("data is too long"));
        }
        nr = if nc == 0 { 0 } else { (lendat_f / nc as f64).ceil() as i64 };
    } else if miss_nc {
        if lendat_f > nr as f64 * i32::MAX as f64 {
            return Err(cx.error("data is too long"));
        }
        nc = if nr == 0 { 0 } else { (lendat_f / nr as f64).ceil() as i64 };
    }
    if lendat > 0 {
        let nrc = nr * nc;
        let ld = lendat as i64;
        if ld > 1 && nrc % ld != 0 {
            if (ld > nr && (ld / nr) * nr != ld) || (ld < nr && (nr / ld) * ld != nr) {
                let m = format!("data length [{ld}] is not a sub-multiple or multiple of the number of rows [{nr}]");
                cx.warning(&m)?;
            } else if (ld > nc && (ld / nc) * nc != ld) || (ld < nc && (nc / ld) * ld != nc) {
                let m = format!("data length [{ld}] is not a sub-multiple or multiple of the number of columns [{nc}]");
                cx.warning(&m)?;
            } else if nrc != ld {
                // (_R_CHECK_MATRIX_DATA_ makes this an error)
                if std::env::var("_R_CHECK_MATRIX_DATA_").is_ok_and(|p| matches!(p.as_str(), "T" | "True" | "TRUE" | "true")) {
                    return Err(cx.error(&format!("data length differs from size of matrix: [{ld} != {nr} x {nc}]")));
                }
                cx.warning(&format!("data length differs from size of matrix: [{ld} != {nr} x {nc}]"))?;
            }
        } else if ld > 1 && nrc == 0 {
            cx.warning("non-empty data for zero-extent matrix")?;
        }
    }
    if (nr as f64) * (nc as f64) > R_XLEN_T_MAX {
        return Err(cx.error("too many elements specified"));
    }
    let (nr, nc) = (nr as usize, nc as usize);
    cx.scope(|cx, s| {
        let ans = s.out(cx);
        let ansr = ans.root();
        cx.build(ans, |cx| {
            let v = vals.bind(cx);
            let n = nr * nc;
            // copyMatrix: by row, or the data recycled in order; no data:
            // NA (raw 0, list NULL)
            let src = |k: usize| -> usize {
                if byrow {
                    let (i, j) = (k % nr, k / nr);
                    (i * nc + j) % lendat
                } else {
                    k % lendat
                }
            };
            // allocMatrix's dim
            let mut a = Attrs::new();
            let dim = cx.alloc_copy(&[nr as i32, nc as i32])?.finish();
            a.set_dim(Attr { tag: cx.install(b"dim")?, value: dim.into() });
            macro_rules! plain {
                ($vec:expr, $na:expr) => {{
                    let xs = $vec.as_slice().unwrap();
                    let mut r = cx.alloc_with(n, |k| if lendat == 0 { $na } else { xs[src(k)] })?;
                    a.apply(cx, &mut r)?;
                    r.finish().into()
                }};
            }
            let r: Ref<'_> = match v.view() {
                View::Lgl(x) => plain!(x, Lgl::NA),
                View::Int(x) => plain!(x, NA_INTEGER),
                View::Real(x) => plain!(x, NA_REAL),
                View::Cplx(x) => plain!(x, Rcomplex { r: NA_REAL, i: 0.0 }),
                View::Raw(x) => plain!(x, 0u8),
                View::Str(x) => {
                    let xs = x.as_slice().unwrap();
                    let mut r = cx.alloc_vector::<Char<'_>>(n)?;
                    for k in 0..n {
                        r.set(k, if lendat == 0 { cx.na_string() } else { xs[src(k)] });
                    }
                    a.apply(cx, &mut r)?;
                    r.finish().into()
                }
                View::List(x) | View::Expr(x) => {
                    let xs = x.as_slice().unwrap();
                    let mut r =
                        if v.sexptype() == EXPRSXP { cx.alloc_expression(n)? } else { cx.alloc_vector::<Ref<'_>>(n)? };
                    if lendat > 0 {
                        for k in 0..n {
                            r.set(k, xs[src(k)]);
                        }
                    }
                    a.apply(cx, &mut r)?;
                    r.finish().into()
                }
                _ => unreachable!("robigo: matrix() of a non-vector"),
            };
            Ok(r)
        })?;
        // dimnamesgets(ans, dimnames) when dimnames has elements
        let dn = args.root(4);
        let dl = dn.bind(cx).len().unwrap_or(0);
        if dn.bind(cx).sexptype() != NILSXP && dl > 0 {
            dimnamesgets(cx, ansr, dn)?;
        }
        out.set(cx, ansr.bind(cx));
        Ok(())
    })
}
