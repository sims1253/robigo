//! relop.c's comparison operators `== != < <= >= >`: `do_relop` and
//! `do_relop_dflt`, over the kernels in [`crate::kernel::relop`].
//!
//! Not here yet, each an error that says so: ordering strings under a
//! collation other than C (GNU R collates with ICU, the root collator in the
//! oracle's `C.UTF-8`), comparing a call (GNU R deparses it), comparing a
//! list (coerceVectorList), and `_R_COMPARE_LANG_OBJECTS` other than its
//! default.

use crate::arith::{MISMATCH, Shape, Side, binary_shape, set_ts};
use crate::attrib::{self, Attrs};
use crate::coerce::Strings;
use crate::kernel::relop::{self as k, NA_LOGICAL, RelOp};
use crate::kernel::{mismatch, result_len, zip2};
use crate::text;
use crate::util::not_yet;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Cx, Dispatched, Generic, Lgl, Out, RResult, Rcomplex, Ref, Root, View};

/// `do_relop`: one instance per operator (`OP` is its names.c code).
// R: relop.c:do_relop
pub fn do_relop<const OP: u8>(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let op = RelOp::from_code(OP);
    let argc = args.len();
    let attr = |cx: &Cx, i: usize| i < argc && attrib::has_attrib(args.get(cx, i));
    let out = if attr(cx, 0) || attr(cx, 1) {
        match cx.try_dispatch(Generic::group("Ops", op.name()), call, args, env, out)? {
            Dispatched::Done => return Ok(()),
            Dispatched::Default(out) => out,
        }
    } else {
        out
    };
    if argc != 2 {
        return Err(cx.error("operator needs two arguments"));
    }
    relop_dflt(cx, op, call, args.root(0), args.root(1), out)
}

/// How the operands compare after `do_relop_dflt`'s coercions.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ty {
    Str,
    Cplx,
    Num,
    Raw,
}

/// `do_relop_dflt(call, op, x, y)`: also what the VM's comparison op
/// falls back to (`cmp_relop` in eval.c).
// R: relop.c:do_relop_dflt
pub fn relop_dflt(cx: &mut Cx, op: RelOp, call: Root<'_>, x: Root<'_>, y: Root<'_>, out: Out<'_>) -> RResult<()> {
    if x.bind(cx).is_package_altrep() || y.bind(cx).is_package_altrep() {
        return Err(not_yet(cx, Some(call), "comparison of a package ALTREP vector"));
    }
    // the simple scalar cases
    if let Some(v) = scalar(op, x.bind(cx), y.bind(cx)) {
        return cx.build(out, |cx| Ok(cx.alloc_copy(&[Lgl(v)])?.finish().into()));
    }
    // symbols and calls (compute_language_relop, its default "eqonly")
    let lang = |r: Ref<'_>| matches!(r.sexptype(), SYMSXP | LANGSXP);
    if lang(x.bind(cx)) || lang(y.bind(cx)) {
        if std::env::var_os("_R_COMPARE_LANG_OBJECTS").is_some_and(|v| v != "eqonly") {
            return Err(not_yet(cx, Some(call), "_R_COMPARE_LANG_OBJECTS other than \"eqonly\""));
        }
        if !matches!(op, RelOp::Eq | RelOp::Ne) {
            let msg = format!("comparison ({}) is not possible for language types", op.name());
            return Err(cx.error_call(Some(call), &msg));
        }
        if x.bind(cx).sexptype() == LANGSXP || y.bind(cx).sexptype() == LANGSXP {
            return Err(not_yet(cx, Some(call), "comparing a call (it deparses the call)"));
        }
    }
    let vec_ok = |r: Ref<'_>| matches!(r.sexptype(), NILSXP | SYMSXP) || attrib::is_vector(r);
    if !vec_ok(x.bind(cx)) || !vec_ok(y.bind(cx)) {
        let msg = format!("comparison ({}) is possible only for atomic and list types", op.name());
        return Err(cx.error_call(Some(call), &msg));
    }
    let list = |r: Ref<'_>| matches!(r.sexptype(), VECSXP | EXPRSXP);
    if list(x.bind(cx)) || list(y.bind(cx)) {
        return Err(not_yet(cx, Some(call), "comparing a list (coerceVectorList)"));
    }
    let (nx, ny, shape, ts, ty) = {
        let (xv, yv) = (x.bind(cx), y.bind(cx));
        let len = |r: Ref<'_>| if r.sexptype() == SYMSXP { 1 } else { r.len().unwrap_or(0) };
        let (nx, ny) = (len(xv), len(yv));
        let (xarray, yarray) = (attrib::is_array(xv), attrib::is_array(yv));
        let shape = if xarray || yarray {
            if xarray && yarray {
                if !attrib::conformable(xv, yv) {
                    return Err(cx.error_call(Some(call), "non-conformable arrays"));
                }
                Shape::Array(Some(Side::X))
            } else if xarray && (ny != 0 || nx == 0) {
                Shape::Array(Some(Side::X))
            } else if yarray && (nx != 0 || ny == 0) {
                Shape::Array(Some(Side::Y))
            } else {
                Shape::Array(None)
            }
        } else {
            Shape::Vector
        };
        let (xts, yts) = (attrib::is_ts(xv), attrib::is_ts(yv));
        if (xts && !yts && nx < ny) || (yts && !xts && ny < nx) {
            return Err(cx.error_call(Some(call), "time-series/vector length mismatch"));
        }
        let ts = if xts {
            Some(Side::X)
        } else if yts {
            Some(Side::Y)
        } else {
            None
        };
        let t = |r: Ref<'_>| r.sexptype();
        let ty = match (t(xv), t(yv)) {
            (STRSXP | SYMSXP, _) | (_, STRSXP | SYMSXP) => Ty::Str,
            (CPLXSXP, _) | (_, CPLXSXP) => Ty::Cplx,
            (RAWSXP, RAWSXP) => Ty::Raw,
            _ => Ty::Num,
        };
        (nx, ny, shape, ts, ty)
    };
    let n = result_len(nx, ny);
    let mut vals: Option<Vec<i32>> = None;
    if n > 0 {
        if mismatch(nx, ny) {
            cx.warning_call(Some(call), MISMATCH)?;
        }
        match ty {
            Ty::Cplx if !matches!(op, RelOp::Eq | RelOp::Ne) => {
                return Err(cx.error_call(Some(call), "invalid comparison with complex values"));
            }
            Ty::Str => match string_relop(cx, op, x.bind(cx), y.bind(cx)) {
                Ok(v) => vals = Some(v),
                Err(e) => return Err(e.raise(cx, call)),
            },
            _ => {}
        }
    }
    let (xattr, yattr) = (attrib::has_attrib(x.bind(cx)), attrib::has_attrib(y.bind(cx)));
    cx.build(out, |cx| {
        let (xv, yv) = (x.bind(cx), y.bind(cx));
        let mut v = cx.alloc_vector::<Lgl>(n)?;
        if n > 0 {
            match (ty, &vals) {
                (Ty::Str, Some(vals)) => v.as_mut_slice().iter_mut().zip(vals).for_each(|(o, &r)| *o = Lgl(r)),
                (Ty::Cplx, _) => cplx_relop(op, xv, yv, v.as_mut_slice()),
                (Ty::Raw, _) => {
                    let (a, b) = (raw(xv), raw(yv));
                    zip2(a, b, v.as_mut_slice(), |p, q| Lgl(k::raw(op, p, q)));
                }
                _ => numeric_relop(op, xv, yv, v.as_mut_slice()),
            }
        }
        if xattr || yattr {
            let mut a = Attrs::new();
            binary_shape(cx, &mut a, shape, None, (true, true), true, xv, yv, n)?;
            if let Some(side) = ts {
                set_ts(cx, &mut a, if side == Side::X { xv } else { yv })?;
            }
            a.apply(cx, &mut v)?;
        }
        Ok(v.finish().into())
    })
}

/// relop.c's `IS_SIMPLE_SCALAR` cases: two attribute-free integer or
/// double scalars.
// R: relop.c:do_relop_dflt
#[inline]
fn scalar(op: RelOp, x: Ref<'_>, y: Ref<'_>) -> Option<i32> {
    let simple = |r: Ref<'_>| r.len() == Some(1) && !attrib::has_attrib(r);
    if !simple(x) || !simple(y) {
        return None;
    }
    match (x.view(), y.view()) {
        (View::Int(a), View::Int(b)) => Some(k::int(op, a.get(0), b.get(0))),
        (View::Int(a), View::Real(b)) => Some(k::int_real(op, a.get(0), b.get(0))),
        (View::Real(a), View::Int(b)) => Some(k::real_int(op, a.get(0), b.get(0))),
        (View::Real(a), View::Real(b)) => Some(k::real(op, a.get(0), b.get(0))),
        _ => None,
    }
}

/// A raw operand's bytes; any other (logical, integer, double) is not raw
/// here: `do_relop_dflt` takes the raw path only for two raw vectors.
fn raw<'g>(x: Ref<'g>) -> &'g [u8] {
    x.as_vector::<u8>().and_then(|v| v.as_slice()).unwrap_or(&[])
}

/// `numeric_relop` over logical, integer, double and raw operands (raw
/// against a number is coerced to that number's type first: the same
/// values).
// R: relop.c:numeric_relop
fn numeric_relop(op: RelOp, x: Ref<'_>, y: Ref<'_>, out: &mut [Lgl]) {
    enum N<'a> {
        I(std::borrow::Cow<'a, [i32]>),
        R(&'a [f64]),
    }
    fn num<'a>(r: Ref<'a>) -> N<'a> {
        match r.view() {
            View::Real(v) => N::R(v.as_slice().unwrap()),
            View::Raw(v) => N::I(v.as_slice().unwrap().iter().map(|&b| b as i32).collect::<Vec<_>>().into()),
            _ => N::I(r.as_ints().unwrap_or(&[]).into()),
        }
    }
    // raw against a logical is coerced to logical: 0/1
    let lgl = |r: Ref<'_>| r.sexptype() == LGLSXP;
    let (a, b) = (num(x), num(y));
    let (a, b) = match (a, b) {
        (N::I(p), N::I(q)) if x.sexptype() == RAWSXP && lgl(y) => {
            (N::I(p.iter().map(|&v| (v != 0) as i32).collect::<Vec<_>>().into()), N::I(q))
        }
        (N::I(p), N::I(q)) if y.sexptype() == RAWSXP && lgl(x) => {
            (N::I(p), N::I(q.iter().map(|&v| (v != 0) as i32).collect::<Vec<_>>().into()))
        }
        other => other,
    };
    match (&a, &b) {
        (N::I(p), N::I(q)) => zip2(p, q, out, |s, t| Lgl(k::int(op, s, t))),
        (N::I(p), N::R(q)) => zip2(p, q, out, |s, t| Lgl(k::int_real(op, s, t))),
        (N::R(p), N::I(q)) => zip2(p, q, out, |s, t| Lgl(k::real_int(op, s, t))),
        (N::R(p), N::R(q)) => zip2(p, q, out, |s, t| Lgl(k::real(op, s, t))),
    }
}

/// `complex_relop` (`==` and `!=`), the operands coerced to complex.
// R: relop.c:complex_relop
fn cplx_relop(op: RelOp, x: Ref<'_>, y: Ref<'_>, out: &mut [Lgl]) {
    let cplx = |r: Ref<'_>| -> Vec<Rcomplex> {
        match r.view() {
            View::Cplx(v) => v.as_slice().unwrap().to_vec(),
            View::Real(v) => v.as_slice().unwrap().iter().map(|&re| Rcomplex { r: re, i: 0.0 }).collect(),
            View::Raw(v) => v.as_slice().unwrap().iter().map(|&b| Rcomplex { r: b as f64, i: 0.0 }).collect(),
            _ => r
                .as_ints()
                .unwrap_or(&[])
                .iter()
                .map(|&i| {
                    if i == NA_LOGICAL {
                        Rcomplex { r: robigo_numerics::NA_REAL, i: robigo_numerics::NA_REAL }
                    } else {
                        Rcomplex { r: i as f64, i: 0.0 }
                    }
                })
                .collect(),
        }
    };
    let (a, b) = (cplx(x), cplx(y));
    zip2(&a, &b, out, |p, q| Lgl(k::complex(op, p, q)));
}

/// Why `string_relop` stopped.
enum StrErr {
    /// a coercion robigo cannot do (the message)
    Coerce(String),
    /// `translateChar` of a "bytes" string, for `Scollate`
    Bytes,
    /// a collation other than C
    Collation,
}

impl StrErr {
    fn raise(self, cx: &mut Cx, call: Root<'_>) -> robigo_rtcore::RError {
        match self {
            StrErr::Coerce(m) => cx.error_call(Some(call), &m),
            StrErr::Bytes => cx.error("translating strings with \"bytes\" encoding is not allowed"),
            StrErr::Collation => {
                not_yet(cx, Some(call), "ordering strings under an ICU collation (LC_COLLATE other than C)")
            }
        }
    }
}

/// `string_relop`, both operands coerced to character: `Seql` for `==`
/// and `!=`, `Scollate` for the orderings.
// R: relop.c:string_relop
fn string_relop(cx: &Cx, op: RelOp, x: Ref<'_>, y: Ref<'_>) -> Result<Vec<i32>, StrErr> {
    let a = Strings::of(cx, x).map_err(|e| StrErr::Coerce(e.message()))?;
    let b = Strings::of(cx, y).map_err(|e| StrErr::Coerce(e.message()))?;
    let loc = cx.locale();
    let n = result_len(a.len(), b.len());
    let mut out = vec![0; n];
    let (n1, n2) = (a.len(), b.len());
    let (mut i1, mut i2) = (0, 0);
    for o in out.iter_mut() {
        *o = match (a.get(i1), b.get(i2)) {
            (None, _) | (_, None) => NA_LOGICAL,
            (Some(s), Some(t)) => match op {
                RelOp::Eq => text::seql(s, t, loc) as i32,
                RelOp::Ne => !text::seql(s, t, loc) as i32,
                _ if a.same_char(i1, &b, i2) => op.on_equal() as i32,
                _ => op.on_order(text::scollate(s, t, loc)?) as i32,
            },
        };
        i1 += 1;
        if i1 == n1 {
            i1 = 0;
        }
        i2 += 1;
        if i2 == n2 {
            i2 = 0;
        }
    }
    Ok(out)
}

impl From<text::CollateError> for StrErr {
    fn from(e: text::CollateError) -> StrErr {
        match e {
            text::CollateError::Bytes => StrErr::Bytes,
            text::CollateError::NotC => StrErr::Collation,
        }
    }
}
