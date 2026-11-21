//! array.c: `.Internal(matrix(data, nrow, ncol, byrow, dimnames,
//! missing(nrow), missing(ncol)))` (`do_matrix`, with duplicate.c's
//! `copyMatrix`; `gc()`'s own R code shapes its result with it), and
//! `.Internal(lengths(x, use.names))` (`do_lengths`), `aperm` (`do_aperm`),
//! `row`/`col` (`do_rowscols`), `asplit` (`do_asplit`), and the matrix
//! products `%*%`, `crossprod`, `tcrossprod` (`do_matprod`).

use crate::attrib::set::dimnamesgets;
use crate::attrib::{self, Attr, Attrs};
use crate::coerce::vector::as_integer_warn;
use crate::kernel::nan::{add, mul};
use crate::util::{as_logical, not_yet, type_to_char};
use robigo_numerics::blas::{self, Trans};
use robigo_rtcore::RSym;
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
/// `INT_MAX` (`do_lengths_long`). A classed `x` and classed elements are
/// read through their `[[` and `length` methods (`dispatch_subset2`,
/// `dispatch_xlength`).
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
    let x = args.root(0);
    // the lengths (as int, then from scratch as double once one passes
    // INT_MAX: do_lengths_long calls the methods again)
    let mut long = false;
    let lens = loop {
        let n = dispatch_xlength(cx, x, call, env)?;
        if n < 0 {
            return Err(cx.error("negative length vectors are not allowed"));
        }
        let n = n as usize;
        let mut lens: Vec<i64> = Vec::with_capacity(n);
        let mut restart = false;
        if is_list {
            for i in 0..n {
                let l = cx.scope(|cx, s| {
                    let e = dispatch_subset2(cx, s, x, i, call, env)?;
                    dispatch_xlength(cx, e, call, env)
                })?;
                if !long && l > i32::MAX as i64 {
                    restart = true;
                    break;
                }
                lens.push(l);
            }
        } else {
            lens.resize(n, 1);
        }
        if restart {
            long = true;
            continue;
        }
        break lens;
    };
    let n = lens.len();
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
            // (int)x_elt_len, NA_INTEGER for a method's NA
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

/// C's `(R_xlen_t) d` as GNU R's x86-64 builds compute it (`cvttsd2si`):
/// truncation toward zero, and `INT64_MIN` for NaN or a value outside the
/// 64-bit range (where Rust's `as` would give 0 or saturate).
fn c_double_to_xlen(d: f64) -> i64 {
    // 2^63, exactly representable
    const LIM: f64 = 9_223_372_036_854_775_808.0;
    if d.is_nan() || d >= LIM || d < -LIM { i64::MIN } else { d as i64 }
}

/// `dispatch_xlength(x, call, rho)`: an object's length through its
/// `length` method (`DispatchOrEval` on `length`; a double result
/// truncated, anything else `asInteger`'d with its warnings), else
/// `xlength(x)`.
// R: array.c:dispatch_xlength
pub fn dispatch_xlength(cx: &mut Cx, x: Root<'_>, call: Root<'_>, rho: Root<'_>) -> RResult<i64> {
    if x.bind(cx).is_object() {
        static LENGTH: Generic = Generic::builtin("length");
        let r = cx.scope(|cx, s| {
            let a = s.args(cx, &[x.bind(cx)], None);
            let o = s.out(cx);
            let res = o.root();
            match cx.try_dispatch(&LENGTH, call, a, rho, o)? {
                Dispatched::Done => {
                    let v = res.bind(cx);
                    Ok(Some(if v.sexptype() == REALSXP {
                        // (R_xlen_t) REAL(len)[0]
                        c_double_to_xlen(v.as_vector::<f64>().unwrap().get(0))
                    } else {
                        as_integer_warn(cx, res)? as i64
                    }))
                }
                Dispatched::Default(_) => Ok(None),
            }
        })?;
        if let Some(n) = r {
            return Ok(n);
        }
    }
    match crate::builtin::length(cx, x.bind(cx)) {
        Some(n) => Ok(n as i64),
        None => Err(not_yet(cx, Some(call), "lengths() of this environment")),
    }
}

/// `dispatch_subset2(x, i, call, rho)`: element `i` of `x`, through `[[`
/// (its method, else the default) for an object, called as
/// `x[[i + 1]]` with a double index.
// R: subset.c:dispatch_subset2
fn dispatch_subset2<'s>(
    cx: &mut Cx,
    s: robigo_rtcore::Scope<'s>,
    x: Root<'_>,
    i: usize,
    call: Root<'_>,
    rho: Root<'_>,
) -> RResult<Root<'s>> {
    if x.bind(cx).is_object() {
        static SUBSET2: Generic = Generic::builtin("[[");
        let idx = s.capture(cx, |cx, o| cx.build_scalar(o, (i + 1) as f64))?;
        let a = s.args(cx, &[x.bind(cx), idx.bind(cx)], None);
        let o = s.out(cx);
        let r = o.root();
        if let Dispatched::Default(o) = cx.try_dispatch(&SUBSET2, call, a, rho, o)? {
            crate::subset::do_subset2_dflt(cx, call, a, rho, o)?;
        }
        Ok(r)
    } else {
        let v = x.bind(cx).as_vector::<Ref<'_>>().expect("robigo: a list").get(i);
        Ok(s.root(cx, v))
    }
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

/// `DropDims(x)`: `x` (fresh, or a copy) without its extents of length
/// one: a vector with the names of the one extent left (or, for one
/// element, of the only extent with names) when at most one remains, else
/// an array of the extents left with their dimnames and the names of both.
/// The attributes are set through `setAttrib`, as GNU R sets them.
// R: array.c:DropDims
pub fn drop_dims(cx: &mut Cx, x: Root<'_>) -> RResult<()> {
    use crate::attrib::set::set_attrib;
    use crate::attrib::{get_value, names};
    let Some(dim) = get_value(x.bind(cx), b"dim").and_then(|d| d.as_ints()).map(<[i32]>::to_vec) else {
        return Ok(());
    };
    let ndims = dim.len();
    let n = dim.iter().filter(|&&d| d != 1).count();
    if n == ndims {
        return Ok(());
    }
    cx.scope(|cx, s| {
        let dimnames = s.root(cx, get_value(x.bind(cx), b"dimnames").unwrap_or_else(|| cx.nil()));
        let nil = s.root(cx, cx.nil());
        // VECTOR_ELT(dimnames, i)
        fn elt<'g>(cx: &'g Cx, dn: Root<'_>, i: usize) -> Ref<'g> {
            match dn.bind(cx).view() {
                View::List(l) => l.get(i),
                _ => cx.nil(),
            }
        }
        if n <= 1 {
            // a vector: if it has length one it is ambiguous which dimnames
            // to use, so use one only if there is only one
            let newnames = {
                let cx = &*cx;
                if dimnames.bind(cx).sexptype() == NILSXP {
                    cx.nil()
                } else if x.bind(cx).len().unwrap_or(0) != 1 {
                    (0..ndims).find(|&i| dim[i] != 1).map_or_else(|| cx.nil(), |i| elt(cx, dimnames, i))
                } else {
                    let set: Vec<usize> = (0..ndims).filter(|&i| elt(cx, dimnames, i).sexptype() != NILSXP).collect();
                    if set.len() == 1 { elt(cx, dimnames, set[0]) } else { cx.nil() }
                }
            };
            let newnames = s.root(cx, newnames);
            set_attrib(cx, x, b"dimnames", nil)?;
            set_attrib(cx, x, b"dim", nil)?;
            return set_attrib(cx, x, b"names", newnames);
        }
        // a lower dimensional array of the n extents left
        let dnn = s.root(cx, names(dimnames.bind(cx)).unwrap_or_else(|| cx.nil()));
        let kept: Vec<usize> = (0..ndims).filter(|&i| dim[i] != 1).collect();
        let newdims = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let v: Vec<i32> = kept.iter().map(|&i| dim[i]).collect();
                Ok(cx.alloc_copy(&v)?.finish().into())
            })
        })?;
        let dim_names = get_value(x.bind(cx), b"dim").and_then(names);
        if dim_names.is_some() {
            let nms = s.capture(cx, |cx, o| {
                cx.build(o, |cx| {
                    let d = get_value(x.bind(cx), b"dim").and_then(names).unwrap();
                    let d = d.as_vector::<Char<'_>>().unwrap();
                    let mut v = cx.alloc_vector::<Char<'_>>(kept.len())?;
                    for (k, &i) in kept.iter().enumerate() {
                        v.set(k, d.get(i));
                    }
                    Ok(v.finish().into())
                })
            })?;
            set_attrib(cx, newdims, b"names", nms)?;
        }
        let havenames =
            dimnames.bind(cx).sexptype() != NILSXP && kept.iter().any(|&i| elt(cx, dimnames, i).sexptype() != NILSXP);
        let newnames = if havenames {
            let l = s.capture(cx, |cx, o| {
                cx.build(o, |cx| {
                    let mut l = cx.alloc_vector::<Ref<'_>>(kept.len())?;
                    for (k, &i) in kept.iter().enumerate() {
                        l.set(k, elt(cx, dimnames, i));
                    }
                    Ok(l.finish().into())
                })
            })?;
            if dnn.bind(cx).sexptype() != NILSXP {
                let nn = s.capture(cx, |cx, o| {
                    cx.build(o, |cx| {
                        let d = dnn.bind(cx).as_vector::<Char<'_>>();
                        let mut v = cx.alloc_vector::<Char<'_>>(kept.len())?;
                        for (k, &i) in kept.iter().enumerate() {
                            v.set(k, d.map_or_else(|| cx.blank_string(), |d| d.get(i)));
                        }
                        Ok(v.finish().into())
                    })
                })?;
                set_attrib(cx, l, b"names", nn)?;
            }
            Some(l)
        } else {
            None
        };
        set_attrib(cx, x, b"dimnames", nil)?;
        set_attrib(cx, x, b"dim", newdims)?;
        if let Some(l) = newnames {
            set_attrib(cx, x, b"dimnames", l)?;
        }
        Ok(())
    })
}

/// `.Internal(drop(x))`: `x` without its extents of length one (a copy if
/// it is referenced).
// R: array.c:do_drop
pub fn do_drop(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("drop", true).check_arity(cx, call, args)?;
    let x = args.root(0);
    let shorten = crate::attrib::get_value(x.bind(cx), b"dim").is_some_and(|d| {
        // INTEGER(xdims): a dim is always an integer vector
        d.as_ints().is_some_and(|v| v.contains(&1))
    });
    if !shorten {
        out.set(cx, x.bind(cx));
        return Ok(());
    }
    cx.scope(|cx, s| {
        let r = if cx.referenced(x.bind(cx)) {
            // R_duplicate_attr: a duplicate here
            s.capture(cx, |cx, o| cx.duplicate(x, o))?
        } else {
            s.root(cx, x.bind(cx))
        };
        drop_dims(cx, r)?;
        out.set(cx, r.bind(cx));
        Ok(())
    })
}

/// `dim2total(dim, &err)`: the product of the extents in `dims` (an error
/// if there are none, or one is NA or negative); None past `R_XLEN_T_MAX`.
// R: array.c:dim2total
pub fn dim2total(cx: &mut Cx, dims: &[i32]) -> RResult<Option<usize>> {
    if dims.is_empty() {
        return Err(cx.error("'dim' cannot be of length 0"));
    }
    let mut dn = 1f64;
    for &d in dims {
        // (NA_INTEGER is negative: tested first)
        if d == NA_INTEGER {
            return Err(cx.error("the dims contain missing values"));
        }
        if d < 0 {
            return Err(cx.error("the dims contain negative values"));
        }
        dn = if d != 0 { dn * d as f64 } else { 0.0 };
    }
    Ok((dn <= R_XLEN_T_MAX).then_some(dn as usize))
}

/// `.Internal(array(data, dim, dimnames))`: `data` recycled (or NA) to the
/// product of `dim`, with that dim and, unless empty, the dimnames.
// R: array.c:do_array
pub fn do_array(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("array", true).check_arity(cx, call, args)?;
    let vals = args.root(0);
    let t = vals.bind(cx).sexptype();
    if !matches!(t, LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP | EXPRSXP | VECSXP) {
        let msg = format!("'data' must be of a vector type, was '{}'", type_to_char(vals.bind(cx)));
        return Err(cx.error(&msg));
    }
    if vals.bind(cx).is_package_altrep() {
        return Err(not_yet(cx, Some(call), "array() of a package ALTREP vector"));
    }
    cx.scope(|cx, s| {
        let dims = s.capture(cx, |cx, o| crate::coerce::coerce_vector(cx, args.root(1), INTSXP, o))?;
        cx.materialize(dims)?;
        cx.materialize(vals)?;
        let d: Vec<i32> = match dims.bind(cx).as_ints() {
            Some(d) => d.to_vec(),
            None => return Err(not_yet(cx, Some(call), "array() with a package ALTREP 'dim'")),
        };
        let Some(nans) = dim2total(cx, &d)? else {
            return Err(cx.error("too many elements specified"));
        };
        let ans = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let v = vals.bind(cx);
                let lendat = v.len().unwrap_or(0);
                macro_rules! fill {
                    ($ty:ty, $na:expr) => {{
                        let xs = v.as_vector::<$ty>().unwrap();
                        let r = if lendat > 0 {
                            cx.alloc_with::<$ty>(nans, |i| xs.get(i % lendat))?
                        } else {
                            cx.alloc_with::<$ty>(nans, |_| $na)?
                        };
                        Ok(r.finish().into())
                    }};
                }
                match t {
                    LGLSXP => fill!(Lgl, Lgl::NA),
                    INTSXP => fill!(i32, NA_INTEGER),
                    REALSXP => fill!(f64, NA_REAL),
                    // (an NA complex here is NA_REAL + 0i)
                    CPLXSXP => fill!(Rcomplex, Rcomplex { r: NA_REAL, i: 0.0 }),
                    RAWSXP => fill!(u8, 0u8),
                    STRSXP => {
                        let xs = v.as_vector::<Char<'_>>().unwrap();
                        let mut r = cx.alloc_vector::<Char<'_>>(nans)?;
                        for i in 0..nans {
                            r.set(i, if lendat > 0 { xs.get(i % lendat) } else { cx.na_string() });
                        }
                        Ok(r.finish().into())
                    }
                    _ => {
                        let xs = v.as_vector::<Ref<'_>>().unwrap();
                        let mut r =
                            if t == EXPRSXP { cx.alloc_expression(nans)? } else { cx.alloc_vector::<Ref<'_>>(nans)? };
                        if lendat > 0 {
                            for i in 0..nans {
                                r.set(i, xs.get(i % lendat));
                            }
                        }
                        Ok(r.finish().into())
                    }
                }
            })
        })?;
        crate::attrib::set::dimgets(cx, ans, dims)?;
        let dn = args.root(2);
        if dn.bind(cx).sexptype() != NILSXP && crate::builtin::length_mut(cx, dn)?.unwrap_or(0) > 0 {
            dimnamesgets(cx, ans, dn)?;
        }
        out.set(cx, ans.bind(cx));
        Ok(())
    })
}

// ---- aperm, row/col, asplit --------------------------------------------------

/// `translateChar(STRING_ELT(x, i))`.
fn translated_elt(cx: &mut Cx, x: Root<'_>, i: usize) -> RResult<Vec<u8>> {
    let r = {
        let c = x.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
        crate::text::to_native(crate::coerce::text(c), cx.locale()).map(|b| b.into_owned())
    };
    r.map_err(|e| cx.error(&e.to_string()))
}

/// A vector of `x`'s type and length `n` whose element `i` is `x`'s
/// element `src(i)` (aperm's and asplit's copies); None for a type the
/// copies do not take.
fn gather<'g>(
    cx: &'g Cx,
    x: Ref<'g>,
    n: usize,
    src: impl Fn(usize) -> usize,
) -> Result<Option<Ref<'g>>, robigo_rtcore::NeedGc> {
    macro_rules! plain {
        ($ty:ty) => {{
            let xs = x.as_vector::<$ty>().unwrap();
            Some(cx.alloc_with::<$ty>(n, |i| xs.get(src(i)))?.finish().into())
        }};
    }
    Ok(match x.sexptype() {
        LGLSXP => plain!(Lgl),
        INTSXP => plain!(i32),
        REALSXP => plain!(f64),
        CPLXSXP => plain!(Rcomplex),
        RAWSXP => plain!(u8),
        STRSXP => {
            let xs = x.as_vector::<Char<'_>>().unwrap();
            let mut r = cx.alloc_vector::<Char<'_>>(n)?;
            for i in 0..n {
                r.set(i, xs.get(src(i)));
            }
            Some(r.finish().into())
        }
        VECSXP => {
            let xs = x.as_vector::<Ref<'_>>().unwrap();
            let mut r = cx.alloc_vector::<Ref<'_>>(n)?;
            for i in 0..n {
                r.set(i, xs.get(src(i)));
            }
            Some(r.finish().into())
        }
        _ => None,
    })
}

/// `.Internal(aperm(a, perm, resize))`: the array `a` with its extents
/// permuted by `perm` (reversed when it is empty; dimension names when it
/// is character), with the permuted dim, its names and the dimnames (and
/// their names) when `resize`, else `a`'s dim.
// R: array.c:do_aperm
pub fn do_aperm(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("aperm", true).check_arity(cx, call, args)?;
    let a = args.root(0);
    if !crate::attrib::is_array(a.bind(cx)) {
        return Err(cx.error("invalid first argument, must be an array"));
    }
    if a.bind(cx).is_package_altrep() {
        return Err(not_yet(cx, Some(call), "aperm() of a package ALTREP array"));
    }
    cx.scope(|cx, s| {
        cx.materialize(a)?;
        let dimsa = s.root(cx, crate::attrib::get_value(a.bind(cx), b"dim").unwrap());
        cx.materialize(dimsa)?;
        let isa: Vec<i32> = dimsa.bind(cx).as_ints().unwrap().to_vec();
        let n = isa.len();
        // check the permutation
        let perm = args.root(1);
        let mut pp = vec![0i64; n];
        if crate::coerce::xlength(perm.bind(cx)) == 0 {
            for (i, p) in pp.iter_mut().enumerate() {
                *p = (n - 1 - i) as i64;
            }
        } else {
            let plen = crate::coerce::xlength(perm.bind(cx));
            if plen != n {
                return Err(cx.error(&format!("'perm' is of wrong length {plen} (!= {n})")));
            }
            if perm.bind(cx).sexptype() == STRSXP {
                let no_names = || "'a' does not have named dimnames";
                let Some(dna) = crate::attrib::get_value(a.bind(cx), b"dimnames") else {
                    return Err(cx.error(no_names()));
                };
                let dna = s.root(cx, dna);
                let Some(dnna) = crate::attrib::get_value(dna.bind(cx), b"names") else {
                    return Err(cx.error(no_names()));
                };
                let dnna = s.root(cx, dnna);
                // translated as the search reaches each name, as GNU R's
                // loop does (stopping at the first match)
                for (i, p) in pp.iter_mut().enumerate() {
                    let this = translated_elt(cx, perm, i)?;
                    let mut found = None;
                    for j in 0..n {
                        if translated_elt(cx, dnna, j)? == this {
                            found = Some(j);
                            break;
                        }
                    }
                    match found {
                        Some(j) => *p = j as i64,
                        None => {
                            let msg = format!("'perm[{}]' does not match a dimension name", i + 1);
                            return Err(cx.error(&msg));
                        }
                    }
                }
            } else {
                let pi = s.capture(cx, |cx, o| crate::coerce::coerce_vector(cx, perm, INTSXP, o))?;
                cx.materialize(pi)?;
                let v = pi.bind(cx).as_ints().unwrap().to_vec();
                for (i, p) in pp.iter_mut().enumerate() {
                    // INTEGER(perm)[i] - 1: NA_INTEGER - 1 in C's int
                    *p = v[i].wrapping_sub(1) as i64;
                }
            }
        }
        let mut iip = vec![0usize; n];
        for &p in &pp {
            if p >= 0 && (p as usize) < n {
                iip[p as usize] += 1;
            } else {
                return Err(cx.error("value out of range in 'perm'"));
            }
        }
        if iip.contains(&0) {
            return Err(cx.error("invalid 'perm' argument"));
        }
        let pp: Vec<usize> = pp.iter().map(|&p| p as usize).collect();
        // the strides, and the dimensions of the result
        if n > 0 {
            iip[0] = 1;
        }
        for i in 1..n {
            iip[i] = iip[i - 1] * isa[i - 1].max(0) as usize;
        }
        let stride: Vec<usize> = pp.iter().map(|&p| iip[p]).collect();
        let isr: Vec<i32> = pp.iter().map(|&p| isa[p]).collect();
        let len = crate::coerce::xlength(a.bind(cx));
        // the source index of each element (CLICKJ)
        let mut src = Vec::with_capacity(len);
        let mut idx = vec![0usize; n];
        let mut lj = 0usize;
        for _ in 0..len {
            src.push(lj);
            for i in 0..n {
                if idx[i] as i64 == isr[i] as i64 - 1 {
                    idx[i] = 0;
                } else {
                    idx[i] += 1;
                    break;
                }
            }
            lj = (0..n).map(|i| idx[i] * stride[i]).sum();
        }
        let o = s.out(cx);
        let r = o.root();
        let mut ok = true;
        cx.build(o, |cx| match gather(cx, a.bind(cx), len, |i| src[i])? {
            Some(v) => Ok(v),
            None => {
                ok = false;
                Ok(cx.nil())
            }
        })?;
        if !ok {
            let msg = format!("unimplemented type '{}' in 'aperm'\n", crate::util::type2char(a.bind(cx).sexptype()));
            return Err(cx.error(&msg));
        }
        // the resize
        let resize = as_logical(cx, args.get(cx, 2));
        if resize == NA_INTEGER {
            return Err(cx.error("'resize' must be TRUE or FALSE"));
        }
        if resize != 0 {
            let dimsr = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.alloc_copy(&isr)?.finish().into())))?;
            if let Some(nmdm) = crate::attrib::get_value(dimsa.bind(cx), b"names") {
                let nmdm = s.root(cx, nmdm);
                let nm = s.capture(cx, |cx, o| {
                    cx.build(o, |cx| {
                        let src = nmdm.bind(cx).as_vector::<Char<'_>>().unwrap();
                        let mut v = cx.alloc_vector::<Char<'_>>(n)?;
                        for (i, &p) in pp.iter().enumerate() {
                            v.set(i, src.get(p));
                        }
                        Ok(v.finish().into())
                    })
                })?;
                crate::attrib::set::set_attrib(cx, dimsr, b"names", nm)?;
            }
            crate::attrib::set::dimgets(cx, r, dimsr)?;
            if let Some(dna) = crate::attrib::get_value(a.bind(cx), b"dimnames") {
                let dna = s.root(cx, dna);
                let dnr = s.capture(cx, |cx, o| {
                    cx.build(o, |cx| {
                        let src = dna.bind(cx).as_vector::<Ref<'_>>().unwrap();
                        let mut v = cx.alloc_vector::<Ref<'_>>(n)?;
                        for (i, &p) in pp.iter().enumerate() {
                            v.set(i, src.get(p));
                        }
                        Ok(v.finish().into())
                    })
                })?;
                if let Some(dnna) = crate::attrib::get_value(dna.bind(cx), b"names") {
                    let dnna = s.root(cx, dnna);
                    let dnnr = s.capture(cx, |cx, o| {
                        cx.build(o, |cx| {
                            let src = dnna.bind(cx).as_vector::<Char<'_>>().unwrap();
                            let mut v = cx.alloc_vector::<Char<'_>>(n)?;
                            for (i, &p) in pp.iter().enumerate() {
                                v.set(i, src.get(p));
                            }
                            Ok(v.finish().into())
                        })
                    })?;
                    crate::attrib::set::set_attrib(cx, dnr, b"names", dnnr)?;
                }
                crate::attrib::set::set_attrib(cx, r, b"dimnames", dnr)?;
            }
        } else {
            crate::attrib::set::dimgets(cx, r, dimsa)?;
        }
        out.set(cx, r.bind(cx));
        Ok(())
    })
}

/// `.Internal(row(dim))` (`CODE` 1) and `.Internal(col(dim))` (2): the
/// integer matrix of the row or column number of each element.
// R: array.c:do_rowscols, allocMatrix
pub fn do_rowscols<const CODE: i32>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let name = if CODE == 2 { "col" } else { "row" };
    let def = if CODE == 2 { crate::row!("col", true) } else { crate::row!("row", true) };
    def.check_arity(cx, call, args)?;
    cx.scope(|cx, s| {
        let d = args.root(0);
        let dim = if d.bind(cx).sexptype() == INTSXP && !crate::attrib::inherits(d.bind(cx), b"factor") {
            d
        } else {
            s.capture(cx, |cx, o| crate::coerce::coerce_vector(cx, d, INTSXP, o))?
        };
        cx.materialize(dim)?;
        if crate::coerce::xlength(dim.bind(cx)) != 2 {
            let msg = format!("a matrix-like object is required as argument to '{name}'");
            return Err(cx.error(&msg));
        }
        let (nr, nc) = {
            let v = dim.bind(cx).as_ints().unwrap();
            (v[0], v[1])
        };
        if nr < 0 || nc < 0 {
            return Err(cx.error("negative extents to matrix"));
        }
        if nr as f64 * nc as f64 > R_XLEN_T_MAX {
            return Err(cx.error("allocMatrix: too many elements specified"));
        }
        let (nr, nc) = (nr as usize, nc as usize);
        let ans = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let v = cx.alloc_with::<i32>(nr * nc, |k| {
                    if CODE == 2 { (k / nr.max(1)) as i32 + 1 } else { (k % nr.max(1)) as i32 + 1 }
                })?;
                Ok(v.finish().into())
            })
        })?;
        let dims =
            s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.alloc_copy(&[nr as i32, nc as i32])?.finish().into())))?;
        crate::attrib::set::dimgets(cx, ans, dims)?;
        out.set(cx, ans.bind(cx));
        Ok(())
    })
}

/// `.Internal(asplit(x, da, dc, dna, dnc, d1, d2, drop))`: `x` (permuted by
/// base's asplit so that each piece is contiguous) cut into `d2` pieces of
/// `d1` elements, a list with dim `da` (and dimnames `dna`), each piece with
/// dim `dc` (and dimnames `dnc`) unless `drop`.
// R: array.c:do_asplit, ASPLIT_ITERATE
pub fn do_asplit(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("asplit", true).check_arity(cx, call, args)?;
    let x = args.root(0);
    if x.bind(cx).is_package_altrep() {
        return Err(not_yet(cx, Some(call), "asplit() of a package ALTREP array"));
    }
    let n1 = crate::coerce::vector::as_integer_warn(cx, args.root(5))?;
    let n2 = crate::coerce::vector::as_integer_warn(cx, args.root(6))?;
    let dnc = args.root(4);
    let havednc = dnc.bind(cx).sexptype() != NILSXP && crate::coerce::xlength(dnc.bind(cx)) > 0;
    // !asLogical(drop): NA is true in C
    let keepdim = as_logical(cx, args.get(cx, 7)) == 0;
    cx.scope(|cx, s| {
        cx.materialize(x)?;
        // allocVector(VECSXP, n2): a negative (or NA) length is an error
        if n2 < 0 {
            return Err(cx.error("negative length vectors are not allowed"));
        }
        let n2 = n2 as usize;
        let y = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.alloc_vector::<Ref<'_>>(n2)?.finish().into())))?;
        crate::attrib::set::dimgets(cx, y, args.root(1))?;
        let dna = args.root(3);
        if dna.bind(cx).sexptype() != NILSXP && crate::coerce::xlength(dna.bind(cx)) > 0 {
            dimnamesgets(cx, y, dna)?;
        }
        // the switch on TYPEOF(x), after y is shaped
        if !matches!(x.bind(cx).sexptype(), LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | VECSXP | RAWSXP) {
            let msg = format!("unimplemented type '{}' in 'asplit'\n", crate::util::type2char(x.bind(cx).sexptype()));
            return Err(cx.error(&msg));
        }
        // each piece's allocVector(TYPEOF(x), n1), inside the loop (none
        // when n2 is 0)
        if n2 > 0 && n1 < 0 {
            return Err(cx.error("negative length vectors are not allowed"));
        }
        let n1 = n1.max(0) as usize;
        if n1 * n2 > crate::coerce::xlength(x.bind(cx)) {
            return Err(not_yet(cx, Some(call), "asplit() past the end of 'x'"));
        }
        for i in 0..n2 {
            let e =
                s.capture(cx, |cx, o| cx.build(o, |cx| Ok(gather(cx, x.bind(cx), n1, |j| i * n1 + j)?.unwrap())))?;
            if keepdim {
                crate::attrib::set::dimgets(cx, e, args.root(2))?;
                if havednc {
                    dimnamesgets(cx, e, dnc)?;
                }
            }
            // (y is fresh: no other reference sees the write)
            cx.set_vector_elt_shared(y, i, e)?;
        }
        out.set(cx, y.bind(cx));
        Ok(())
    })
}

/// `.Internal(t.default(a))`: the transpose of a matrix, or of a vector
/// (one column, `names` its row names) or a 1-d array; the dimnames (and
/// their names) swapped, every other attribute but names kept.
// R: array.c:do_transpose
pub fn do_transpose(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("t.default", true).check_arity(cx, call, args)?;
    let a = args.root(0);
    let av = a.bind(cx);
    if !crate::attrib::is_vector(av) {
        return Err(cx.error("argument is not a matrix"));
    }
    if av.is_package_altrep() {
        return Err(not_yet(cx, Some(call), "t() of a package ALTREP vector"));
    }
    let ldim = crate::attrib::dims(av).map_or(0, |d| d.len());
    if ldim > 2 {
        return Err(cx.error("argument is not a matrix"));
    }
    if !matches!(av.sexptype(), LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | VECSXP | RAWSXP) {
        return Err(cx.error("argument is not a matrix"));
    }
    cx.materialize(a)?;
    cx.build(out, |cx| {
        let a = a.bind(cx);
        let len = a.len().unwrap();
        // (nrow, ncol, rnames, cnames, dimnames' names, any dimnames)
        let (nrow, ncol) = match crate::attrib::dims(a) {
            Some(&[r, c]) => (r as usize, c as usize),
            _ => (len, 1),
        };
        let dn = match ldim {
            0 => None,
            _ => crate::attrib::get_value(a, b"dimnames"),
        };
        let dn_list = dn.and_then(|d| match d.view() {
            View::List(l) => Some(l),
            _ => None,
        });
        let (rnames, cnames) = match (ldim, dn_list) {
            (0, _) => (crate::attrib::names(a), None),
            (1, Some(l)) => (Some(l.get(0)), None),
            (2, Some(l)) => (Some(l.get(0)), Some(l.get(1))),
            _ => (None, None),
        };
        let dnn = dn.and_then(crate::attrib::names);
        // !isNull(dimnames): the names of a vector, a 1-d or 2-d array's
        // dimnames
        let any_dimnames = if ldim == 0 { rnames.is_some() } else { dn.is_some() };
        // filling in columnwise, "accessing row-wise"
        // (the element read now, then j += nrow wrapped past len - 1)
        let order = |j: &mut usize| {
            let r = *j;
            *j += nrow;
            if len > 0 && *j > len - 1 {
                *j -= len - 1;
            }
            r
        };
        macro_rules! plain {
            ($t:ty, $v:expr) => {{
                let src = $v.as_slice().unwrap();
                let mut r = cx.alloc_vector::<$t>(len)?;
                let o = r.as_mut_slice();
                let mut j = 0;
                for o in o.iter_mut() {
                    *o = src[order(&mut j)];
                }
                r
            }};
        }
        let mut at = Attrs::new();
        let dims = cx.alloc_copy(&[ncol as i32, nrow as i32])?.finish().into();
        at.install(Attr { tag: cx.sym(robigo_rtcore::RSym::Dim), value: dims });
        if any_dimnames {
            let mut d = cx.alloc_vector::<Ref<'_>>(2)?;
            d.set(0, cnames.unwrap_or(cx.nil()));
            d.set(1, rnames.unwrap_or(cx.nil()));
            if let Some(dnn) = dnn.and_then(|n| n.as_vector::<Char<'_>>()) {
                let mut nn = cx.alloc_vector::<Char<'_>>(2)?;
                nn.set(1, dnn.get(0));
                nn.set(0, if ldim == 2 { dnn.get(1) } else { cx.blank_string() });
                let mut na = Attrs::new();
                na.install(Attr { tag: cx.sym(robigo_rtcore::RSym::Names), value: nn.finish().into() });
                na.apply(cx, &mut d)?;
            }
            at.install(Attr { tag: cx.sym(robigo_rtcore::RSym::DimNames), value: d.finish().into() });
        }
        at.copy_most(a);
        macro_rules! done {
            ($r:expr) => {{
                let mut r = $r;
                at.apply(cx, &mut r)?;
                Ok(r.finish().into())
            }};
        }
        match a.view() {
            View::Lgl(v) => done!(plain!(Lgl, v)),
            View::Int(v) => done!(plain!(i32, v)),
            View::Real(v) => done!(plain!(f64, v)),
            View::Cplx(v) => done!(plain!(Rcomplex, v)),
            View::Raw(v) => done!(plain!(u8, v)),
            View::Str(v) => {
                let mut r = cx.alloc_vector::<Char<'_>>(len)?;
                let mut j = 0;
                for i in 0..len {
                    r.set(i, v.get(order(&mut j)));
                }
                done!(r)
            }
            View::List(v) => {
                let mut r = cx.alloc_vector::<Ref<'_>>(len)?;
                let mut j = 0;
                for i in 0..len {
                    r.set(i, v.get(order(&mut j)));
                }
                done!(r)
            }
            _ => unreachable!("robigo: t.default's types were checked"),
        }
    })
}

// ---- %*%, crossprod, tcrossprod ---------------------------------------------
//
// array.c's matrix products: `x %*% y` (`do_matprod`, PRIMVAL 0),
// `crossprod(x, y)` (1) and `tcrossprod(x, y)` (2), primitives that dispatch
// on the `matrixOps` group. With `options(matprod = "default")` (the only
// value robigo-rt's options take yet: `options(matprod = )` is not
// implemented) a product whose operands pass `mayHaveNaNOrInf` (no NaN, no
// infinity, no two neighbours summing to one) goes through the reference
// BLAS (`robigo_numerics::blas`), any other through array.c's simple loops,
// in `double` (array.c's `LDOUBLE` loops are `matprod = "internal"`'s).

/// The `matrixOps` group generic of each product.
struct MatrixOps<const OP: i32>;

impl<const OP: i32> MatrixOps<OP> {
    const GENERIC: Generic = Generic::group("matrixOps", name(OP));
}

const fn name(op: i32) -> &'static str {
    match op {
        0 => "%*%",
        1 => "crossprod",
        _ => "tcrossprod",
    }
}

/// `mayHaveNaNOrInf(x, n)`: TRUE when x has a NaN or an infinity, and
/// also when two neighbours sum to one (1e308 + 1e308).
// R: array.c:mayHaveNaNOrInf
fn may_have_nan_or_inf(x: &[f64]) -> bool {
    let n = x.len();
    if n & 1 != 0 && !x[0].is_finite() {
        return true;
    }
    let mut i = n & 1;
    while i < n {
        if !(x[i] + x[i + 1]).is_finite() {
            return true;
        }
        i += 2;
    }
    false
}

/// `cmayHaveNaNOrInf(x, n)`.
// R: array.c:cmayHaveNaNOrInf
fn cmay_have_nan_or_inf(x: &[Rcomplex]) -> bool {
    let n = x.len();
    if n & 1 != 0 && (!x[0].r.is_finite() || !x[0].i.is_finite()) {
        return true;
    }
    let mut i = n & 1;
    while i < n {
        if !(x[i].r + x[i].i + x[i + 1].r + x[i + 1].i).is_finite() {
            return true;
        }
        i += 2;
    }
    false
}

/// The C99 `xij * yjk` of the simple complex loops (libgcc's
/// `__muldc3`).
#[inline(always)]
fn cmul(a: Rcomplex, b: Rcomplex) -> Rcomplex {
    crate::kernel::complex::cmul(a, b)
}

/// One of the three products' simple loops (array.c's `MATPROD_BODY`,
/// `CROSSPROD_BODY`, `TCROSSPROD_BODY` in `double`): `z[i, k]` the sum
/// over `j` of `x[xi(i, j)] * y[yi(j, k)]`, for `i < ni`, `k < nk`,
/// `j < nj`, stored at `i + k * ldz`.
#[allow(clippy::too_many_arguments)]
fn simple(
    x: &[f64],
    y: &[f64],
    z: &mut [f64],
    (ni, nk, nj, ldz): (usize, usize, usize, usize),
    xi: impl Fn(usize, usize) -> usize,
    yi: impl Fn(usize, usize) -> usize,
) {
    for i in 0..ni {
        for k in 0..nk {
            let mut sum = 0.0;
            for j in 0..nj {
                sum = add(sum, mul(x[xi(i, j)], y[yi(j, k)]));
            }
            z[i + k * ldz] = sum;
        }
    }
}

/// The complex simple loops (`CMATPROD_BODY` and friends): real and
/// imaginary parts summed apart, each of C99's product.
fn csimple(
    x: &[Rcomplex],
    y: &[Rcomplex],
    z: &mut [Rcomplex],
    (ni, nk, nj, ldz): (usize, usize, usize, usize),
    xi: impl Fn(usize, usize) -> usize,
    yi: impl Fn(usize, usize) -> usize,
) {
    for i in 0..ni {
        for k in 0..nk {
            let (mut sr, mut si) = (0.0, 0.0);
            for j in 0..nj {
                let p = cmul(x[xi(i, j)], y[yi(j, k)]);
                sr = add(sr, p.r);
                si = add(si, p.i);
            }
            z[i + k * ldz] = Rcomplex { r: sr, i: si };
        }
    }
}

/// `matprod`, `crossprod`, `tcrossprod` (and `symcrossprod`,
/// `symtcrossprod` when `sym`) of double matrices into `z`.
// R: array.c:matprod, crossprod, tcrossprod, symcrossprod, symtcrossprod
#[allow(clippy::too_many_arguments)]
fn real_product(
    op: i32,
    sym: bool,
    x: &[f64],
    nrx: usize,
    ncx: usize,
    y: &[f64],
    nry: usize,
    ncy: usize,
    z: &mut [f64],
) {
    if nrx == 0 || ncx == 0 || nry == 0 || ncy == 0 {
        // zero-extent operations should return zeroes
        z.fill(0.0);
        return;
    }
    let nonfinite = may_have_nan_or_inf(x) || (!sym && may_have_nan_or_inf(y));
    match (op, sym) {
        (0, _) => {
            if nonfinite {
                simple(x, y, z, (nrx, ncy, ncx, nrx), |i, j| i + j * nrx, |j, k| j + k * nry);
            } else if ncy == 1 {
                // matrix-vector or dot product
                blas::dgemv(false, nrx, ncx, x, nrx, y, z);
            } else if nrx == 1 {
                // vector-matrix: (Y^T)(x^T)
                blas::dgemv(true, nry, ncy, y, nry, x, z);
            } else {
                blas::dgemm(Trans::NN, nrx, ncy, ncx, x, nrx, y, nry, z, nrx);
            }
        }
        (1, true) => {
            let (nr, nc) = (nrx, ncx);
            if nonfinite {
                simple(x, x, z, (nc, nc, nr, nc), |i, j| j + i * nr, |j, k| j + k * nr);
            } else {
                blas::dsyrk_upper(true, nc, nr, x, nr, z, nc);
                for i in 1..nc {
                    for j in 0..i {
                        z[i + nc * j] = z[j + nc * i];
                    }
                }
            }
        }
        (1, false) => {
            if nonfinite {
                simple(x, y, z, (ncx, ncy, nrx, ncx), |i, j| j + i * nrx, |j, k| j + k * nry);
            } else if ncy == 1 {
                blas::dgemv(true, nrx, ncx, x, nrx, y, z);
            } else if ncx == 1 {
                // vector-matrix: (Y^T)x
                blas::dgemv(true, nry, ncy, y, nry, x, z);
            } else {
                blas::dgemm(Trans::TN, ncx, ncy, nrx, x, nrx, y, nry, z, ncx);
            }
        }
        (_, true) => {
            let (nr, nc) = (nrx, ncx);
            if nonfinite {
                simple(x, x, z, (nr, nr, nc, nr), |i, j| i + j * nr, |j, k| k + j * nr);
            } else {
                blas::dsyrk_upper(false, nr, nc, x, nr, z, nr);
                for i in 1..nr {
                    for j in 0..i {
                        z[i + nr * j] = z[j + nr * i];
                    }
                }
            }
        }
        (_, false) => {
            if nonfinite {
                simple(x, y, z, (nrx, nry, ncx, nrx), |i, j| i + j * nrx, |j, k| k + j * nry);
            } else if nry == 1 {
                blas::dgemv(false, nrx, ncx, x, nrx, y, z);
            } else if nrx == 1 {
                // vector-matrix: Y(x^T)
                blas::dgemv(false, nry, ncy, y, nry, x, z);
            } else {
                blas::dgemm(Trans::NT, nrx, nry, ncx, x, nrx, y, nry, z, nrx);
            }
        }
    }
}

/// `cmatprod`, `ccrossprod`, `tccrossprod` into `z` (the symmetric
/// forms are the general ones of x with itself).
// R: array.c:cmatprod, ccrossprod, tccrossprod
#[allow(clippy::too_many_arguments)]
fn complex_product(
    op: i32,
    x: &[Rcomplex],
    nrx: usize,
    ncx: usize,
    y: &[Rcomplex],
    nry: usize,
    ncy: usize,
    z: &mut [Rcomplex],
) {
    if nrx == 0 || ncx == 0 || nry == 0 || ncy == 0 {
        z.fill(Rcomplex { r: 0.0, i: 0.0 });
        return;
    }
    let nonfinite = cmay_have_nan_or_inf(x) || cmay_have_nan_or_inf(y);
    match op {
        0 if nonfinite => csimple(x, y, z, (nrx, ncy, ncx, nrx), |i, j| i + j * nrx, |j, k| j + k * nry),
        0 => blas::zgemm(Trans::NN, (nrx, ncy, ncx), x, nrx, y, nry, z, nrx, |c| (c.r, c.i), |r, i| Rcomplex { r, i }),
        1 if nonfinite => csimple(x, y, z, (ncx, ncy, nrx, ncx), |i, j| j + i * nrx, |j, k| j + k * nry),
        1 => blas::zgemm(Trans::TN, (ncx, ncy, nrx), x, nrx, y, nry, z, ncx, |c| (c.r, c.i), |r, i| Rcomplex { r, i }),
        _ if nonfinite => csimple(x, y, z, (nrx, nry, ncx, nrx), |i, j| i + j * nrx, |j, k| k + j * nry),
        _ => blas::zgemm(Trans::NT, (nrx, nry, ncx), x, nrx, y, nry, z, nrx, |c| (c.r, c.i), |r, i| Rcomplex { r, i }),
    }
}

/// `x %*% y` (`OP` 0), `crossprod(x, y)` (1), `tcrossprod(x, y)` (2).
// R: array.c:do_matprod
pub fn do_matprod<const OP: i32>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    // .Primitive(); may have 1 or 2 args, but some methods have more
    let nargs = args.len().min(2);
    let min_nargs = if OP != 0 { 1 } else { 2 };
    if nargs < min_nargs {
        let s = if nargs == 1 { "" } else { "s" };
        let msg = format!("{nargs} argument{s} passed to '{}' which requires at least {min_nargs}", name(OP));
        return Err(cx.error_call(Some(call), &msg));
    }
    let x0 = args.get(cx, 0);
    let y0 = if args.len() > 1 { args.get(cx, 1) } else { cx.nil() };
    let out = if x0.is_object() || y0.is_object() {
        // Remove argument names to ensure positional matching
        let d = cx.scope(|cx, s| {
            let vals: Vec<Ref<'_>> = (0..args.len()).map(|i| args.get(cx, i)).collect();
            let plain = s.args(cx, &vals, None);
            cx.try_dispatch(&MatrixOps::<OP>::GENERIC, call, plain, env, out)
        })?;
        match d {
            Dispatched::Done => return Ok(()),
            Dispatched::Default(out) => out,
        }
    } else {
        out
    };
    // the default method
    if args.len() > 2 {
        let msg = format!("more than 2 arguments passed to default method of '{}'", name(OP));
        cx.warning_call(Some(call), &msg)?;
    }
    // y = x for crossprod(x) and tcrossprod(x); x %*% NULL is refused below
    let sym = args.len() < 2 || args.get(cx, 1).sexptype() == NILSXP;
    let (xr, yr) = (args.root(0), if sym && OP > 0 { args.root(0) } else { args.root(1) });
    let ok = |v: Ref<'_>| is_numeric(v) || v.sexptype() == CPLXSXP;
    if !ok(xr.bind(cx)) || !ok(yr.bind(cx)) {
        return Err(cx.error_call(Some(call), "requires numeric/complex matrix/vector arguments"));
    }
    if xr.bind(cx).is_package_altrep() || yr.bind(cx).is_package_altrep() {
        return Err(not_yet(cx, Some(call), &format!("{}() of a package ALTREP vector", name(OP))));
    }
    let shape = |v: Ref<'_>| attrib::dims(v).map(|d| d.to_vec()).unwrap_or_default();
    let (xdims, ydims) = (shape(xr.bind(cx)), shape(yr.bind(cx)));
    let (ldx, ldy) = (xdims.len(), ydims.len());
    let (lx, ly) = (xr.bind(cx).len().unwrap(), yr.bind(cx).len().unwrap());
    let (nrx, ncx, nry, ncy) = dims(OP, &xdims, &ydims, lx, ly);
    match OP {
        // primitive, so use call
        0 if ncx != nry => return Err(cx.error_call(Some(call), "non-conformable arguments")),
        1 if nrx != nry => return Err(cx.error("non-conformable arguments")),
        2 if ncx != ncy => return Err(cx.error("non-conformable arguments")),
        _ => {}
    }
    let complex = args.get(cx, 0).sexptype() == CPLXSXP || (args.len() > 1 && args.get(cx, 1).sexptype() == CPLXSXP);
    let mode = if complex { CPLXSXP } else { REALSXP };
    cx.scope(|cx, s| {
        let x = s.capture(cx, |cx, o| crate::coerce::vector::coerce_vector(cx, xr, mode, o))?;
        let y = if sym { x } else { s.capture(cx, |cx, o| crate::coerce::vector::coerce_vector(cx, yr, mode, o))? };
        cx.materialize(x)?;
        cx.materialize(y)?;
        let (ar, ac) = match OP {
            0 => (nrx, ncy),
            1 => (ncx, ncy),
            _ => (nrx, nry),
        };
        cx.build(out, |cx| {
            let (xv, yv) = (x.bind(cx), y.bind(cx));
            let mut a = Attrs::new();
            let d = cx.alloc_copy(&[ar as i32, ac as i32])?.finish().into();
            a.install(Attr { tag: cx.sym(RSym::Dim), value: d });
            if let Some(dn) = dimnames(cx, OP, sym, xv, yv, (ldx, ldy, ncx, nry))? {
                a.install(Attr { tag: cx.sym(RSym::DimNames), value: dn });
            }
            match (xv.view(), yv.view()) {
                (View::Real(xs), View::Real(ys)) => {
                    let mut r = cx.alloc_vector::<f64>(ar * ac)?;
                    let (xs, ys) = (xs.as_slice().unwrap(), ys.as_slice().unwrap());
                    real_product(OP, sym && OP != 0, xs, nrx, ncx, ys, nry, ncy, r.as_mut_slice());
                    a.apply(cx, &mut r)?;
                    Ok(r.finish().into())
                }
                (View::Cplx(xs), View::Cplx(ys)) => {
                    let mut r = cx.alloc_vector::<Rcomplex>(ar * ac)?;
                    let (xs, ys) = (xs.as_slice().unwrap(), ys.as_slice().unwrap());
                    complex_product(OP, xs, nrx, ncx, ys, nry, ncy, r.as_mut_slice());
                    a.apply(cx, &mut r)?;
                    Ok(r.finish().into())
                }
                _ => unreachable!("robigo: matprod's operands were coerced"),
            }
        })
    })
}

/// The extents do_matprod gives x and y: (nrx, ncx, nry, ncy), a vector
/// taking the shape that makes the product conformable where GNU R lets it.
// R: array.c:do_matprod
fn dims(op: i32, xd: &[i32], yd: &[i32], lx: usize, ly: usize) -> (usize, usize, usize, usize) {
    let d = |v: &[i32], i: usize| v[i] as usize;
    if xd.len() != 2 && yd.len() != 2 {
        // x and y non-matrices; for crossprod, allow two cases: n x n ==>
        // (1,n) x (n,1); 1 x n = (n, 1) x (1, n)
        if op == 1 && lx == 1 {
            (1, 1, 1, ly)
        } else {
            let (mut nry, mut ncy) = (ly, 1);
            if op == 0 {
                let ncx = lx;
                if ncx == 1 {
                    // y as row vector
                    ncy = nry;
                    nry = 1;
                }
                (1, ncx, nry, ncy)
            } else {
                (lx, 1, nry, ncy)
            }
        }
    } else if xd.len() != 2 {
        // x not a matrix
        let (nry, ncy) = (d(yd, 0), d(yd, 1));
        let (mut nrx, mut ncx) = (0, 0);
        match op {
            0 => {
                if lx == nry {
                    (nrx, ncx) = (1, nry);
                } else if nry == 1 {
                    (nrx, ncx) = (lx, 1);
                }
            }
            1 => {
                if lx == nry {
                    (nrx, ncx) = (nry, 1);
                }
            }
            _ => {
                if lx == ncy {
                    (nrx, ncx) = (1, ncy);
                } else if ncy == 1 {
                    (nrx, ncx) = (lx, 1);
                }
            }
        }
        (nrx, ncx, nry, ncy)
    } else if yd.len() != 2 {
        // y not a matrix
        let (nrx, ncx) = (d(xd, 0), d(xd, 1));
        let (mut nry, mut ncy) = (0, 0);
        match op {
            0 => {
                if ly == ncx {
                    (nry, ncy) = (ncx, 1);
                } else if ncx == 1 {
                    (nry, ncy) = (1, ly);
                }
            }
            1 => {
                if ly == nrx {
                    (nry, ncy) = (nrx, 1);
                } else if nrx == 1 {
                    (nry, ncy) = (1, ly);
                }
            }
            _ => {
                if nrx == 1 {
                    (nry, ncy) = (1, ly);
                } else {
                    (nry, ncy) = (ly, 1);
                }
            }
        }
        (nrx, ncx, nry, ncy)
    } else {
        (d(xd, 0), d(xd, 1), d(yd, 0), d(yd, 1))
    }
}

/// The product's dimnames (`ALLOC_DIMNAMES_NAMES`, `YDIMS_ET_CETERA`,
/// `SET_DIMNAMES_NAMES`): None when both would be NULL. `x` and `y` are
/// the coerced operands (coerceVector keeps their attributes).
// R: array.c:do_matprod
fn dimnames<'g>(
    cx: &'g Cx,
    op: i32,
    sym: bool,
    x: Ref<'g>,
    y: Ref<'g>,
    (ldx, ldy, ncx, nry): (usize, usize, usize, usize),
) -> Result<Option<Ref<'g>>, robigo_rtcore::NeedGc> {
    let xdims = attrib::get_value(x, b"dimnames");
    let ydims = if sym && op != 0 { xdims } else { attrib::get_value(y, b"dimnames") };
    if xdims.is_none() && ydims.is_none() {
        return Ok(None);
    }
    let elt = |dn: Ref<'g>, i: usize| match dn.view() {
        View::List(l) if i < l.len() => l.get(i),
        _ => cx.nil(),
    };
    let nm = |dn: Ref<'g>, i: usize| -> Option<Char<'g>> {
        attrib::names(dn).and_then(|n| n.as_vector::<Char<'g>>()).map(|n| n.get(i))
    };
    let (mut d0, mut d1) = (cx.nil(), cx.nil());
    let (mut n0, mut n1): (Option<Char<'g>>, Option<Char<'g>>) = (None, None);
    let (mut dnx, mut dny) = (false, false);
    if let Some(xd) = xdims {
        let pick = match op {
            0 if ldx == 2 || ncx == 1 => Some(0),
            1 if ldx == 2 => Some(1),
            2 if ldx == 2 => Some(0),
            _ => None,
        };
        if let Some(i) = pick {
            d0 = elt(xd, i);
            if attrib::names(xd).is_some() {
                dnx = true;
                n0 = nm(xd, i);
            }
        }
    }
    if let Some(yd) = ydims {
        let pick = match op {
            2 if ldy == 2 => Some(0),
            2 => None,
            _ if ldy == 2 => Some(1),
            _ if nry == 1 => Some(0),
            _ => None,
        };
        if let Some(i) = pick {
            d1 = elt(yd, i);
            if attrib::names(yd).is_some() {
                dny = true;
                n1 = nm(yd, i);
            }
        }
    }
    // Since R 2.1.0, no longer attach a dimnames attribute whose elements
    // are all NULL
    if d0.sexptype() == NILSXP && d1.sexptype() == NILSXP {
        return Ok(None);
    }
    let mut dn = cx.alloc_vector::<Ref<'g>>(2)?;
    dn.set(0, d0);
    dn.set(1, d1);
    if dnx || dny {
        let mut names = cx.alloc_vector::<Char<'g>>(2)?;
        names.set(0, n0.unwrap_or(cx.blank_string()));
        names.set(1, n1.unwrap_or(cx.blank_string()));
        let mut a = Attrs::new();
        a.install(Attr { tag: cx.sym(RSym::Names), value: names.finish().into() });
        a.apply(cx, &mut dn)?;
    }
    Ok(Some(dn.finish().into()))
}
