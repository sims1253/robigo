//! arithmetic.c's operators `+ - * / ^ %% %/%`: `do_arith`, `R_binary`,
//! `R_unary`, and the attribute rules of `integer_binary`, `real_binary`
//! and complex.c's `complex_binary`. The element work is the kernels'
//! ([`crate::kernel::arith`], [`crate::kernel::complex`]).
//!
//! Not here yet, each noted where GNU R does it: reusing an argument's
//! storage for the result (`R_allocOrReuseVector`, `NO_REFERENCES`: R1b's
//! `cx.unique_mut` makes that possible), so a result is always a fresh
//! vector; package ALTREP arguments (an error says so).

use crate::attrib::{self, Attr, Attrs};
use crate::kernel::arith::{self as k, ArithOp, RealArg};
use crate::kernel::complex as kc;
use crate::kernel::{mismatch, result_len};
use crate::util::not_yet;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Built, Cx, Dispatched, FreshVec, Generic, NeedGc, Out, RResult, Rcomplex, Ref, Root, View};

/// GNU R's recycling warning.
pub(crate) const MISMATCH: &str = "longer object length is not a multiple of shorter object length";
const OVERFLOW: &str = "NAs produced by integer overflow";
const LOST: &str = "probable complete loss of accuracy in modulus";

/// `do_arith`: the `+`, `-`, `*`, `/`, `^`, `%%` and `%/%` builtins, one
/// instance per operator (`OP` is its names.c code).
// R: arithmetic.c:do_arith
pub fn do_arith<const OP: u8>(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let op = ArithOp::from_code(OP);
    let argc = args.len();
    let attr = |cx: &Cx, i: usize| i < argc && attrib::has_attrib(args.get(cx, i));
    let plain = !(attr(cx, 0) || attr(cx, 1));
    let out = if plain {
        out
    } else {
        match cx.try_dispatch(&Ops::<OP>::GENERIC, call, args, env, out)? {
            Dispatched::Done => return Ok(()),
            Dispatched::Default(out) => out,
        }
    };
    if args.any_package_altrep() {
        return Err(not_yet(cx, Some(call), "arithmetic on a package ALTREP vector"));
    }
    match argc {
        2 => {
            if plain {
                // do_arith's scalar operations
                match scalar(cx, op, args.get(cx, 0), args.get(cx, 1)) {
                    Some(Scalar::Real(v)) => return cx.build(out, |cx| Ok(cx.alloc_copy(&[v])?.finish().into())),
                    Some(Scalar::Int(v, naflag)) => {
                        cx.build(out, |cx| Ok(cx.alloc_copy(&[v])?.finish().into()))?;
                        return if naflag { cx.warning_call(Some(call), OVERFLOW) } else { Ok(()) };
                    }
                    None => {}
                }
            }
            r_binary(cx, op, call, args.root(0), args.root(1), out)
        }
        1 => r_unary(cx, op, call, args.root(0), out),
        _ => Err(cx.error_call(Some(call), "operator needs one or two arguments")),
    }
}

enum Scalar {
    Real(f64),
    Int(i32, bool),
}

/// do_arith's fast path for two attribute-free scalars of type double or
/// integer and `+ - * /`.
// R: arithmetic.c:do_arith
#[inline]
fn scalar(cx: &Cx, op: ArithOp, x: Ref<'_>, y: Ref<'_>) -> Option<Scalar> {
    let _ = cx;
    if !matches!(op, ArithOp::Plus | ArithOp::Minus | ArithOp::Times | ArithOp::Div) {
        return None;
    }
    let one = |r: Ref<'_>| r.len() == Some(1);
    if !one(x) || !one(y) {
        return None;
    }
    let real = |v: f64, w: f64, x_dest: bool| Scalar::Real(k::real_op(op, v, w, x_dest));
    match (x.view(), y.view()) {
        (View::Real(a), View::Real(b)) => Some(real(a.get(0), b.get(0), true)),
        (View::Real(a), View::Int(b)) => Some(real(a.get(0), k::int_to_real(b.get(0)), false)),
        (View::Int(a), View::Real(b)) => Some(real(k::int_to_real(a.get(0)), b.get(0), false)),
        (View::Int(a), View::Int(b)) => {
            let (i1, i2) = (a.get(0), b.get(0));
            let mut naflag = false;
            Some(match op {
                ArithOp::Plus => Scalar::Int(k::int_plus(i1, i2, &mut naflag), naflag),
                ArithOp::Minus => Scalar::Int(k::int_minus(i1, i2, &mut naflag), naflag),
                ArithOp::Times => Scalar::Int(k::int_times(i1, i2, &mut naflag), naflag),
                _ => Scalar::Real(k::int_divide(i1, i2)),
            })
        }
        _ => None,
    }
}

/// Which argument an attribute comes from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Side {
    X,
    Y,
}

/// The result type of `R_binary`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ty {
    Int,
    Real,
    Cplx,
}

/// What `R_binary` decides from its arguments' types, lengths and
/// attributes before it computes: plain data, so the warnings and errors
/// can be signalled (`&mut Cx`) in GNU R's order around the computation.
struct Plan {
    nx: usize,
    ny: usize,
    xattr: bool,
    yattr: bool,
    /// the array-of-length-1 special: the argument lost dim and dimnames
    strip: Option<Side>,
    /// `R_binary`'s `dims`, and the argument the dimnames (or names) come
    /// from in the array branch
    shape: Shape,
    ts: Option<Side>,
    s4: bool,
    ty: Ty,
}

#[derive(Clone, Copy)]
pub(crate) enum Shape {
    /// `xarray || yarray`: dim from this argument (None: R_NilValue)
    Array(Option<Side>),
    /// names
    Vector,
}

/// `R_binary`: two numeric (or NULL) arguments.
// R: arithmetic.c:R_binary
fn r_binary(cx: &mut Cx, op: ArithOp, call: Root<'_>, x: Root<'_>, y: Root<'_>, out: Out<'_>) -> RResult<()> {
    // FIXUP_NULL_AND_CHECK_TYPES
    for r in [x, y] {
        if !matches!(r.bind(cx).sexptype(), NILSXP | CPLXSXP | REALSXP | INTSXP | LGLSXP) {
            return Err(cx.error_call(Some(call), "non-numeric argument to binary operator"));
        }
    }
    let plan = {
        let (xv, yv) = (x.bind(cx), y.bind(cx));
        // admitted: do_arith answered package ALTREP arguments, and the types
        // were checked numeric or NULL above
        let (nx, ny) = (admitted(xv).len(), admitted(yv).len());
        let (xattr, yattr) = (attrib::has_attrib(xv), attrib::has_attrib(yv));
        let (xarray, yarray) = (xattr && attrib::is_array(xv), yattr && attrib::is_array(yv));
        let ty = match (xv.sexptype(), yv.sexptype()) {
            (CPLXSXP, _) | (_, CPLXSXP) => Ty::Cplx,
            (REALSXP, _) | (_, REALSXP) => Ty::Real,
            _ => Ty::Int,
        };
        Plan {
            nx,
            ny,
            xattr,
            yattr,
            strip: None,
            shape: Shape::Vector,
            ts: None,
            s4: (xattr && xv.is_s4()) || (yattr && yv.is_s4()),
            ty,
        }
        .arrays(xarray, yarray)
    };
    // R_ARITHMETIC_ARRAY_1_SPECIAL: a length-1 array against a vector of
    // another length loses its dim (and dimnames); deprecated with a warning
    let mut plan = plan;
    {
        let (xv, yv) = (x.bind(cx), y.bind(cx));
        let (xarray, yarray) = (plan.xattr && attrib::is_array(xv), plan.yattr && attrib::is_array(yv));
        if xarray != yarray {
            if xarray && plan.nx == 1 && plan.ny != 1 {
                if plan.ny != 0 {
                    cx.warning_call(
                        Some(call),
                        "Recycling array of length 1 in array-vector arithmetic is deprecated.\n  Use c() or as.vector() instead.",
                    )?;
                }
                plan.strip = Some(Side::X);
            }
            if yarray && plan.ny == 1 && plan.nx != 1 {
                if plan.nx != 0 {
                    cx.warning_call(
                        Some(call),
                        "Recycling array of length 1 in vector-array arithmetic is deprecated.\n  Use c() or as.vector() instead.",
                    )?;
                }
                plan.strip = Some(Side::Y);
            }
        }
    }
    if let (Shape::Array(_), true) = (plan.shape, plan.strip.is_none()) {
        let (xv, yv) = (x.bind(cx), y.bind(cx));
        if attrib::is_array(xv) && attrib::is_array(yv) && !attrib::conformable(xv, yv) {
            return Err(cx.error_call(Some(call), "non-conformable arrays"));
        }
    }
    // time series: the shorter one is an error
    {
        let (xv, yv) = (x.bind(cx), y.bind(cx));
        let (xts, yts) = (plan.xattr && attrib::is_ts(xv), plan.yattr && attrib::is_ts(yv));
        plan.ts = match (xts, yts) {
            (true, _) => Some(Side::X),
            (false, true) => Some(Side::Y),
            _ => None,
        };
        if (xts && !yts && plan.nx < plan.ny) || (yts && !xts && plan.ny < plan.nx) {
            return Err(cx.error_call(Some(call), "time-series/vector length mismatch"));
        }
    }
    if mismatch(plan.nx, plan.ny) {
        cx.warning_call(Some(call), MISMATCH)?;
    }
    let n = result_len(plan.nx, plan.ny);
    if plan.ty == Ty::Cplx && matches!(op, ArithOp::Mod | ArithOp::IDiv) && n > 0 {
        return Err(cx.error("unimplemented complex operation"));
    }
    let late = late_error(cx, &plan, op, x, y, n);
    // the integer overflow flag and the count of accuracy losses come back
    // from the attempt that succeeded: its warnings follow, in GNU R's order
    let (naflag, lost) = cx.build_with(out, |cx| {
        let (xv, yv) = (x.bind(cx), y.bind(cx));
        let (mut naflag, mut lost) = (false, 0usize);
        let r: Ref<'_> = match plan.ty {
            Ty::Int if op.int_gives_real() => {
                let mut v = cx.alloc_vector::<f64>(n)?;
                k::int_binary_real(op, ints(xv), ints(yv), v.as_mut_slice());
                finish(cx, &plan, op, late.is_none(), v, xv, yv)?
            }
            Ty::Int => {
                let mut v = cx.alloc_vector::<i32>(n)?;
                naflag = k::int_binary(op, ints(xv), ints(yv), v.as_mut_slice());
                finish(cx, &plan, op, late.is_none(), v, xv, yv)?
            }
            Ty::Real => {
                let mut v = cx.alloc_vector::<f64>(n)?;
                let int_operand = xv.sexptype() == INTSXP || yv.sexptype() == INTSXP;
                lost = k::real_binary(op, real_arg(xv), real_arg(yv), int_operand, v.as_mut_slice());
                finish(cx, &plan, op, late.is_none(), v, xv, yv)?
            }
            Ty::Cplx => {
                let (a, b) = (cplx(xv), cplx(yv));
                let mut v = cx.alloc_vector::<Rcomplex>(n)?;
                let f = match op {
                    ArithOp::Plus => kc::cadd,
                    ArithOp::Minus => kc::csub,
                    ArithOp::Times => kc::ctimes,
                    ArithOp::Div => kc::cdiv,
                    ArithOp::Pow => kc::cpow,
                    // empty (checked above)
                    ArithOp::Mod | ArithOp::IDiv => kc::cadd,
                };
                crate::kernel::zip2(&a, &b, v.as_mut_slice(), f);
                finish(cx, &plan, op, late.is_none(), v, xv, yv)?
            }
        };
        Ok(Built::Done(r, (naflag, lost)))
    })?;
    if naflag {
        cx.warning_call(Some(call), OVERFLOW)?;
    }
    for _ in 0..lost {
        cx.warning(LOST)?;
    }
    match late {
        Some(msg) => Err(cx.error(msg)),
        None => Ok(()),
    }
}

/// An operand of `R_binary`/`R_unary` past `do_arith`'s checks (numeric or
/// NULL, and no package ALTREP argument): a vector in standard storage.
fn admitted(r: Ref<'_>) -> robigo_rtcore::StdVec<'_> {
    r.std_vector().expect("robigo: an arithmetic operand is a numeric vector or NULL in standard storage")
}

impl Plan {
    /// `R_binary`'s choice of `dims`: from both arrays (checked
    /// conformable), or from the array whose partner is not empty.
    fn arrays(mut self, xarray: bool, yarray: bool) -> Plan {
        if xarray || yarray {
            self.shape = Shape::Array(if xarray && yarray {
                Some(Side::X)
            } else if xarray && (self.ny != 0 || self.nx == 0) {
                Some(Side::X)
            } else if yarray && (self.nx != 0 || self.ny == 0) {
                Some(Side::Y)
            } else {
                None
            });
        }
        self
    }
}

/// The errors GNU R raises after computing, setting the result's
/// attributes: `tspgets` (a time series of the wrong length) and
/// `classgets` (a factor class on a non-integer). With `error()`, so they
/// come after the computation's warnings.
fn late_error(cx: &Cx, plan: &Plan, op: ArithOp, x: Root<'_>, y: Root<'_>, n: usize) -> Option<&'static str> {
    let int_result = plan.ty == Ty::Int && !op.int_gives_real();
    ts_late_error(plan.shape, plan.strip, plan.ts, x.bind(cx), y.bind(cx), n, int_result)
}

/// [`ts_error`] for a binary result of length `n` whose dim (if any) comes
/// from `shape`, its time series attributes from `ts`.
pub(crate) fn ts_late_error(
    shape: Shape,
    strip: Option<Side>,
    ts: Option<Side>,
    x: Ref<'_>,
    y: Ref<'_>,
    n: usize,
    int_result: bool,
) -> Option<&'static str> {
    let of = |s: Side| if s == Side::X { x } else { y };
    let src = of(ts?);
    // nrows(val): the first extent once dim is set, else the length
    let nrows = match shape {
        Shape::Array(Some(d)) if strip != Some(d) => {
            attrib::dims(of(d)).and_then(|d| d.first().map(|&e| e as usize)).unwrap_or(n)
        }
        _ => n,
    };
    ts_error(src, nrows, int_result)
}

/// `tspgets` and `classgets` on a result of `nrows` rows from the time
/// series `src`: GNU R's message, if they fail.
// R: attrib.c:tspgets, attrib.c:classgets
pub(crate) fn ts_error(src: Ref<'_>, nrows: usize, int_result: bool) -> Option<&'static str> {
    let tsp = attrib::get_value(src, b"tsp").and_then(|t| t.as_vector::<f64>()).and_then(|v| v.as_slice());
    if let Some(&[start, end, freq]) = tsp {
        if nrows == 0 {
            return Some("cannot assign 'tsp' to zero-length vector");
        }
        if freq <= 0.0 || (end - start - (nrows as f64 - 1.0) / freq).abs() > 1.0e-5 {
            return Some("invalid time series parameters specified");
        }
    }
    let factor = attrib::get_value(src, b"class")
        .and_then(|c| c.as_vector::<robigo_rtcore::Char<'_>>())
        .is_some_and(|v| v.as_slice().unwrap().iter().any(|c| c.bytes() == b"factor"));
    if !int_result && factor {
        return Some("adding class \"factor\" to an invalid object");
    }
    None
}

/// The `*_binary` and `R_binary` attribute rules on a fresh result.
fn finish<'g, T>(
    cx: &'g Cx,
    plan: &Plan,
    op: ArithOp,
    ts_ok: bool,
    mut v: FreshVec<'g, T>,
    x: Ref<'g>,
    y: Ref<'g>,
) -> Result<Ref<'g>, NeedGc>
where
    T: robigo_rtcore::Elt<'g>,
{
    let _ = op;
    if !(plan.xattr || plan.yattr) {
        return Ok(v.finish().into());
    }
    let n = v.len();
    let mut a = Attrs::new();
    // *_binary: "copy attributes from longer argument", y's first so that
    // x's overwrite them (dim and dimnames never come along); an empty
    // result returns before that
    if n > 0 && n == plan.ny && attrib::has_attrib(y) {
        a.copy_most(y);
    }
    if n > 0 && n == plan.nx && attrib::has_attrib(x) {
        a.copy_most(x);
    }
    binary_shape(cx, &mut a, plan.shape, plan.strip, (plan.xattr, plan.yattr), false, x, y, n)?;
    if ts_ok && let Some(side) = plan.ts {
        let src = if side == Side::X { x } else { y };
        set_ts(cx, &mut a, src)?;
    }
    if plan.s4 {
        a.set_s4();
    }
    a.apply(cx, &mut v)?;
    Ok(v.finish().into())
}

/// `R_binary`/`do_relop_dflt`/`lbinary`'s shape attributes on a result of
/// length `n`: `dim` and `dimnames`, or `names`. `R_binary` installs the
/// first names whose length is `n`'s (`XLENGTH(val) == xlength(xnames)`,
/// a missing one counting as length 0); `do_relop_dflt` and `lbinary` the
/// first that exists and has `n`'s length (`present_first`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn binary_shape<'g>(
    cx: &'g Cx,
    a: &mut Attrs<'g>,
    shape: Shape,
    strip: Option<Side>,
    (xattr, yattr): (bool, bool),
    present_first: bool,
    x: Ref<'g>,
    y: Ref<'g>,
    n: usize,
) -> Result<(), NeedGc> {
    let of = |s: Side| if s == Side::X { x } else { y };
    let present = |s: Side| if s == Side::X { xattr } else { yattr };
    match shape {
        Shape::Array(dims) => {
            // dimnames read from both (a stripped argument has none)
            let dn = |s: Side| (present(s) && strip != Some(s)).then(|| attrib::get(of(s), b"dimnames")).flatten();
            if let Some(d) = dims.filter(|&d| strip != Some(d))
                && let Some(dim) = attrib::get(of(d), b"dim")
            {
                a.set_dim(dim);
                if let Some(names) = dn(Side::X).or_else(|| dn(Side::Y)) {
                    a.install(names);
                }
            } else {
                // dims is R_NilValue: names from the dimnames read above
                let pick = pick_names(present_first, n, dn(Side::X).map(|a| a.value), dn(Side::Y).map(|a| a.value));
                if let Some(nm) = pick {
                    a.install(Attr { tag: cx.install(b"names")?, value: nm });
                }
            }
        }
        Shape::Vector => {
            let names = |s: Side| present(s).then(|| attrib::names(of(s))).flatten();
            if let Some(nm) = pick_names(present_first, n, names(Side::X), names(Side::Y)) {
                a.install(Attr { tag: cx.install(b"names")?, value: nm });
            }
        }
    }
    Ok(())
}

/// Which names a binary result of length `n` takes ([`binary_shape`]).
fn pick_names<'g>(present_first: bool, n: usize, x: Option<Ref<'g>>, y: Option<Ref<'g>>) -> Option<Ref<'g>> {
    if present_first {
        x.filter(|v| attrib::len(Some(*v)) == n).or_else(|| y.filter(|v| attrib::len(Some(*v)) == n))
    } else if n == attrib::len(x) {
        x
    } else if n == attrib::len(y) {
        y
    } else {
        None
    }
}

/// `setAttrib(val, R_TspSymbol, tsp)` (`tspgets` installs a fresh copy)
/// then `setAttrib(val, R_ClassSymbol, klass)`, from the time series `src`.
pub(crate) fn set_ts<'g>(cx: &'g Cx, a: &mut Attrs<'g>, src: Ref<'g>) -> Result<(), NeedGc> {
    if let Some(t) = attrib::get(src, b"tsp") {
        let vals = t.value.as_vector::<f64>().and_then(|v| v.as_slice()).unwrap_or(&[]);
        let copy = cx.alloc_copy(vals)?.finish();
        a.install(Attr { tag: t.tag, value: copy.into() });
    }
    let klass = attrib::get_value(src, b"class");
    a.set_class(cx.install(b"class")?, klass);
    Ok(())
}

/// A logical, integer or NULL operand's elements as integers.
fn ints<'g>(x: Ref<'g>) -> &'g [i32] {
    x.as_ints().unwrap_or(&[])
}

/// An operand as `real_binary` reads it: doubles, or integers (and
/// logicals, which GNU R coerces to double first: the same values).
fn real_arg<'g>(x: Ref<'g>) -> RealArg<'g> {
    match x.as_vector::<f64>() {
        Some(v) => RealArg::Real(v.as_slice().unwrap()),
        None => RealArg::Int(ints(x)),
    }
}

/// An operand coerced to complex (`ComplexFromInteger`,
/// `ComplexFromLogical`, `ComplexFromReal`): an NA is NA in the real part
/// and 0 in the imaginary (GNU R is built without `NA_TO_COMPLEX_NA`).
// R: coerce.c:ComplexFromInteger, coerce.c:ComplexFromReal
fn cplx<'g>(x: Ref<'g>) -> std::borrow::Cow<'g, [Rcomplex]> {
    use std::borrow::Cow;
    match x.view() {
        View::Cplx(v) => Cow::Borrowed(v.as_slice().unwrap()),
        View::Real(v) => Cow::Owned(v.as_slice().unwrap().iter().map(|&r| Rcomplex { r, i: 0.0 }).collect()),
        _ => Cow::Owned(
            ints(x)
                .iter()
                .map(|&i| {
                    if i == k::NA_INTEGER {
                        Rcomplex { r: k::NA_REAL, i: 0.0 }
                    } else {
                        Rcomplex { r: i as f64, i: 0.0 }
                    }
                })
                .collect(),
        ),
    }
}

/// `R_unary`: `+x` and `-x`.
// R: arithmetic.c:R_unary
fn r_unary(cx: &mut Cx, op: ArithOp, call: Root<'_>, x: Root<'_>, out: Out<'_>) -> RResult<()> {
    let t = x.bind(cx).sexptype();
    match (t, op) {
        (LGLSXP, ArithOp::Plus | ArithOp::Minus) => cx.build(out, |cx| logical_unary(cx, op, x.bind(cx))),
        (INTSXP | REALSXP | CPLXSXP, ArithOp::Plus) => {
            // returns its argument itself
            out.set(cx, x.bind(cx));
            Ok(())
        }
        (INTSXP | REALSXP | CPLXSXP, ArithOp::Minus) => cx.build(out, |cx| negate(cx, x.bind(cx))),
        (CPLXSXP, _) => Err(cx.error_call(Some(call), "invalid complex unary operator")),
        (LGLSXP | INTSXP | REALSXP, _) => Err(cx.error_call(Some(call), "invalid unary operator")),
        _ => Err(cx.error_call(Some(call), "invalid argument to unary operator")),
    }
}

/// `logical_unary`: an integer result with the argument's names, dim and
/// dimnames.
// R: arithmetic.c:logical_unary
fn logical_unary<'g>(cx: &'g Cx, op: ArithOp, x: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
    let xs = ints(x);
    let mut v = cx.alloc_with(xs.len(), |i| if op == ArithOp::Minus { k::int_neg(xs[i]) } else { xs[i] })?;
    let mut a = Attrs::new();
    names_dim_dimnames(cx, &mut a, x)?;
    a.apply(cx, &mut v)?;
    Ok(v.finish().into())
}

/// `setAttrib` of `x`'s names, dim and dimnames, in that order (as
/// `logical_unary` and `lunary` copy them).
pub(crate) fn names_dim_dimnames<'g>(cx: &'g Cx, a: &mut Attrs<'g>, x: Ref<'g>) -> Result<(), NeedGc> {
    if !attrib::has_attrib(x) {
        return Ok(());
    }
    if let Some(nm) = attrib::names(x) {
        a.install(Attr { tag: cx.install(b"names")?, value: nm });
    }
    if let Some(d) = attrib::get(x, b"dim") {
        a.set_dim(d);
    }
    if let Some(dn) = attrib::get(x, b"dimnames") {
        a.install(dn);
    }
    Ok(())
}

/// `-x` for an integer, double or complex vector: `duplicate(x)` negated,
/// every attribute kept.
// R: arithmetic.c:integer_unary, arithmetic.c:real_unary, complex.c:complex_unary
fn negate<'g>(cx: &'g Cx, x: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
    fn with_attrs<'g, T: robigo_rtcore::Elt<'g>>(
        cx: &'g Cx,
        x: Ref<'g>,
        mut v: FreshVec<'g, T>,
    ) -> Result<Ref<'g>, NeedGc> {
        let mut a = Attrs::new();
        a.copy_all(x);
        a.apply(cx, &mut v)?;
        Ok(v.finish().into())
    }
    match x.view() {
        View::Int(xs) => {
            let xs = xs.as_slice().unwrap();
            with_attrs(cx, x, cx.alloc_with(xs.len(), |i| k::int_neg(xs[i]))?)
        }
        View::Real(xs) => {
            let xs = xs.as_slice().unwrap();
            with_attrs(cx, x, cx.alloc_with(xs.len(), |i| -xs[i])?)
        }
        View::Cplx(xs) => {
            let xs = xs.as_slice().unwrap();
            with_attrs(cx, x, cx.alloc_with(xs.len(), |i| Rcomplex { r: -xs[i].r, i: -xs[i].i })?)
        }
        _ => unreachable!("robigo: negate of a non-number"),
    }
}

/// The `Ops` group generic of the operator with names.c code `OP`.
struct Ops<const OP: u8>;

impl<const OP: u8> Ops<OP> {
    const GENERIC: Generic = Generic::group("Ops", ArithOp::from_code(OP).name());
}
