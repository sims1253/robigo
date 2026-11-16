//! array.c: `.Internal(matrix(data, nrow, ncol, byrow, dimnames,
//! missing(nrow), missing(ncol)))` (`do_matrix`, with duplicate.c's
//! `copyMatrix`; `gc()`'s own R code shapes its result with it), and
//! `.Internal(lengths(x, use.names))` (`do_lengths`).

use crate::attrib::set::dimnamesgets;
use crate::attrib::{Attr, Attrs};
use crate::coerce::vector::as_integer_warn;
use crate::util::{as_logical, not_yet, type_to_char};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Dispatched, Generic, Lgl, Out, RResult, Rcomplex, Ref, Root, View};

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
                if std::env::var("_R_CHECK_MATRIX_DATA_")
                    .is_ok_and(|p| matches!(p.as_str(), "T" | "True" | "TRUE" | "true"))
                {
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

const LENGTHS: Generic = Generic::builtin("lengths");

/// `.Internal(lengths(x, use.names))`: each element's length, or 1 for
/// every element of an atomic vector, with `x`'s dim and, for `use.names`,
/// its names and dimnames; a double vector when a length passes
/// `INT_MAX` (`do_lengths_long`). GNU R reads a classed `x` and classed
/// elements through `[[` and `length` methods (`dispatch_subset2`,
/// `dispatch_xlength`): not implemented yet.
// R: array.c:do_lengths, getElementLength, do_lengths_long
pub fn do_lengths(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("lengths").check_arity(cx, call, args)?;
    let use_names = as_logical(cx, args.get(cx, 1));
    if use_names == NA_INTEGER {
        return Err(cx.error("invalid 'use.names' value"));
    }
    let Dispatched::Default(out) = crate::coerce::dispatch_or_eval(cx, &LENGTHS, call, args, env, out)? else {
        return Ok(());
    };
    let x = args.get(cx, 0);
    let is_list = matches!(x.sexptype(), VECSXP | EXPRSXP) || x.is_s4();
    if !is_list && !matches!(x.sexptype(), NILSXP | CHARSXP | LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP) {
        return Err(cx.error("'x' must be a list or atomic vector"));
    }
    if x.is_object() {
        return Err(crate::util::not_yet(
            cx,
            Some(call),
            "lengths() of an object (dispatch_subset2, dispatch_xlength)",
        ));
    }
    let n = crate::coerce::xlength(x);
    let mut lens = Vec::with_capacity(n);
    if is_list {
        let elts = x.as_vector::<Ref<'_>>().expect("robigo: a list");
        for i in 0..n {
            let e = elts.get(i);
            if e.is_object() {
                return Err(crate::util::not_yet(cx, Some(call), "lengths() of a classed element (dispatch_xlength)"));
            }
            match crate::builtin::length(cx, e) {
                Some(l) => lens.push(l),
                None => return Err(crate::util::not_yet(cx, Some(call), "lengths() of this environment")),
            }
        }
    } else {
        lens.resize(n, 1);
    }
    let long = lens.iter().any(|&l| l > i32::MAX as usize);
    let use_names = use_names != 0;
    // the result as allocVector makes one, of any length: through
    // `alloc_rooted`, which collects first if due (docs/g3.md §3 item 3), and
    // rooted in `out` before its attributes are set
    let ans = out.root();
    if long {
        let mut r = cx.alloc_rooted::<f64>(n)?;
        for (d, &l) in r.as_mut_slice().iter_mut().zip(&lens) {
            *d = l as f64;
        }
        let c = r.cx();
        out.set(c, r.finish());
    } else {
        let mut r = cx.alloc_rooted::<i32>(n)?;
        for (d, &l) in r.as_mut_slice().iter_mut().zip(&lens) {
            *d = l as i32;
        }
        let c = r.cx();
        out.set(c, r.finish());
    }
    // setAttrib(ans, R_DimSymbol, dim), then with use.names the names and
    // dimnames (a one-dimensional result's names are its dimnames:
    // namesgets, which the dimnames step then replaces)
    let x = args.root(0);
    let mut which: Vec<&[u8]> = vec![b"dim"];
    if use_names {
        which.extend([b"names".as_slice(), b"dimnames"]);
    }
    cx.scope(|cx, s| {
        for name in which {
            let v = s.out(cx);
            let vr = v.root();
            cx.build(v, |cx| Ok(crate::attrib::get_attrib(cx, x.bind(cx), name)?.unwrap_or(cx.nil())))?;
            if vr.bind(cx).sexptype() != NILSXP {
                crate::attrib::set::set_attrib(cx, ans, name, vr)?;
            }
        }
        Ok(())
    })
}

/// `.Internal(colSums(x, n, p, na.rm))` (`CODE` 0), `colMeans` (1),
/// `rowSums` (2), `rowMeans` (3): over the `n` by `p` matrix in `x`'s
/// first `n * p` elements, in long double.
// R: array.c:do_colsum
pub fn do_colsum<const CODE: i32>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let row = [
        crate::row!("colSums", true),
        crate::row!("colMeans", true),
        crate::row!("rowSums", true),
        crate::row!("rowMeans", true),
    ][CODE as usize];
    row.check_arity(cx, call, args)?;
    // asVecSize: its errors, and -999 (or a negative size) invalid
    let size = |cx: &mut Cx, i: usize, what: &str| -> RResult<usize> {
        match crate::builtin::as_vec_size(cx, args, i)? {
            Ok(Some(k)) => Ok(k),
            Ok(None) => Err(cx.error(&format!("invalid '{what}' argument"))),
            Err(m) => Err(cx.error(m)),
        }
    };
    let n = size(cx, 1, "n")?;
    let p = size(cx, 2, "p")?;
    let narm = crate::util::as_logical(cx, args.get(cx, 3));
    if narm == i32::MIN {
        return Err(cx.error("invalid 'na.rm' argument"));
    }
    let keep_na = narm == 0;
    let x = args.get(cx, 0);
    let t = x.sexptype();
    if !matches!(t, LGLSXP | INTSXP | REALSXP | CPLXSXP) {
        return Err(cx.error("'x' must be numeric or complex"));
    }
    if (n as f64) * (p as f64) > crate::coerce::xlength(x) as f64 {
        return Err(cx.error("'x' is too short"));
    }
    if x.is_package_altrep() {
        return Err(not_yet(cx, Some(call), "colSums() of a package ALTREP vector"));
    }
    use robigo_numerics::ldouble::LDouble as LD;
    let na_real = robigo_numerics::NA_REAL;
    let mean = CODE == 1 || CODE == 3;
    // a column (or row) of values as the C reads it: (real, imaginary or
    // None, is NA)
    enum Data<'a> {
        Real(&'a [f64]),
        Int(Vec<i32>),
        Cplx(&'a [Rcomplex]),
    }
    let data = match x.view() {
        View::Real(v) => Data::Real(v.as_slice().unwrap()),
        View::Int(v) => Data::Int(v.as_slice().unwrap().to_vec()),
        View::Lgl(v) => Data::Int(v.as_slice().unwrap().iter().map(|l| l.0).collect()),
        View::Cplx(v) => Data::Cplx(v.as_slice().unwrap()),
        _ => unreachable!(),
    };
    let (mut re, mut im): (Vec<f64>, Vec<f64>) = (Vec::new(), Vec::new());
    if CODE <= 1 {
        for j in 0..p {
            let off = n * j;
            match &data {
                Data::Real(x) => {
                    let (mut sum, mut cnt) = (LD::ZERO, n);
                    if keep_na {
                        for &v in &x[off..off + n] {
                            sum += v;
                        }
                    } else {
                        cnt = 0;
                        for &v in &x[off..off + n] {
                            if !v.is_nan() {
                                cnt += 1;
                                sum += v;
                            }
                        }
                    }
                    if mean {
                        sum /= cnt as f64;
                    }
                    re.push(sum.to_f64());
                }
                Data::Int(x) => {
                    let (mut sum, mut cnt) = (LD::ZERO, 0usize);
                    for &v in &x[off..off + n] {
                        if v != i32::MIN {
                            cnt += 1;
                            sum += v as f64;
                        } else if keep_na {
                            sum = LD::from_f64(na_real);
                            break;
                        }
                    }
                    if mean {
                        sum /= cnt as f64;
                    }
                    re.push(sum.to_f64());
                }
                Data::Cplx(x) => {
                    let (mut sr, mut si, mut cnt) = (LD::ZERO, LD::ZERO, n);
                    if keep_na {
                        for z in &x[off..off + n] {
                            sr += z.r;
                            si += z.i;
                        }
                    } else {
                        cnt = 0;
                        for z in &x[off..off + n] {
                            if !z.r.is_nan() && !z.i.is_nan() {
                                cnt += 1;
                                sr += z.r;
                                si += z.i;
                            }
                        }
                    }
                    if mean {
                        sr /= cnt as f64;
                        si /= cnt as f64;
                    }
                    re.push(sr.to_f64());
                    im.push(si.to_f64());
                }
            }
        }
    } else {
        let (mut rans, mut ians) = (vec![LD::ZERO; n], vec![LD::ZERO; n]);
        let mut cnt = vec![0usize; n];
        for j in 0..p {
            let off = n * j;
            for i in 0..n {
                match &data {
                    Data::Real(x) => {
                        let v = x[off + i];
                        if keep_na {
                            rans[i] += v;
                        } else if !v.is_nan() {
                            rans[i] += v;
                            cnt[i] += 1;
                        }
                    }
                    Data::Int(x) => {
                        let v = x[off + i];
                        if keep_na {
                            if v != i32::MIN {
                                rans[i] += v as f64;
                            } else {
                                rans[i] = LD::from_f64(na_real);
                            }
                        } else if v != i32::MIN {
                            rans[i] += v as f64;
                            cnt[i] += 1;
                        }
                    }
                    Data::Cplx(x) => {
                        let z = x[off + i];
                        if keep_na {
                            rans[i] += z.r;
                            ians[i] += z.i;
                        } else if !z.r.is_nan() && !z.i.is_nan() {
                            rans[i] += z.r;
                            ians[i] += z.i;
                            cnt[i] += 1;
                        }
                    }
                }
            }
        }
        if CODE == 3 {
            for i in 0..n {
                let d = if keep_na { p as f64 } else { cnt[i] as f64 };
                rans[i] /= d;
                ians[i] /= d;
            }
        }
        re = rans.iter().map(|v| v.to_f64()).collect();
        im = ians.iter().map(|v| v.to_f64()).collect();
    }
    cx.build(out, |cx| {
        Ok(if t == CPLXSXP {
            let z: Vec<Rcomplex> = re.iter().zip(&im).map(|(&r, &i)| Rcomplex { r, i }).collect();
            cx.alloc_copy(&z)?.finish().into()
        } else {
            cx.alloc_copy(&re)?.finish().into()
        })
    })
}
