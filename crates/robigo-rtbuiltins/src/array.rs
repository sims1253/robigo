//! array.c: `.Internal(matrix(data, nrow, ncol, byrow, dimnames,
//! missing(nrow), missing(ncol)))` (`do_matrix`, with duplicate.c's
//! `copyMatrix`; `gc()`'s own R code shapes its result with it), and
//! `.Internal(lengths(x, use.names))` (`do_lengths`), `aperm` (`do_aperm`),
//! `row`/`col` (`do_rowscols`) and `asplit` (`do_asplit`).

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
                let names: Vec<Vec<u8>> = (0..n).map(|j| translated_elt(cx, dnna, j)).collect::<RResult<_>>()?;
                for (i, p) in pp.iter_mut().enumerate() {
                    let this = translated_elt(cx, perm, i)?;
                    match names.iter().position(|nm| *nm == this) {
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
    let n1 = crate::coerce::vector::as_integer(cx, args.get(cx, 5)).0;
    let n2 = crate::coerce::vector::as_integer(cx, args.get(cx, 6)).0;
    let dnc = args.root(4);
    let havednc = dnc.bind(cx).sexptype() != NILSXP && crate::coerce::xlength(dnc.bind(cx)) > 0;
    // !asLogical(drop): NA is true in C
    let keepdim = as_logical(cx, args.get(cx, 7)) == 0;
    cx.scope(|cx, s| {
        cx.materialize(x)?;
        let (n1, n2) = (n1.max(0) as usize, n2.max(0) as usize);
        if !matches!(x.bind(cx).sexptype(), LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | VECSXP | RAWSXP) {
            // (y is made before the type is looked at, as in GNU R; nothing
            // to observe in between)
            let msg = format!("unimplemented type '{}' in 'asplit'\n", crate::util::type2char(x.bind(cx).sexptype()));
            return Err(cx.error(&msg));
        }
        if n1 * n2 > crate::coerce::xlength(x.bind(cx)) {
            return Err(not_yet(cx, Some(call), "asplit() past the end of 'x'"));
        }
        let y = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.alloc_vector::<Ref<'_>>(n2)?.finish().into())))?;
        crate::attrib::set::dimgets(cx, y, args.root(1))?;
        let dna = args.root(3);
        if dna.bind(cx).sexptype() != NILSXP && crate::coerce::xlength(dna.bind(cx)) > 0 {
            dimnamesgets(cx, y, dna)?;
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
