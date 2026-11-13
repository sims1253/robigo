//! logic.c's `&`, `|` and `!` (`do_logic`, `lbinary`, `lunary`) over the
//! kernels in [`crate::kernel::logic`]. `&&` and `||` are the VM's (they
//! are language specials, docs/g3.md §10 "Designed away").

use crate::arith::{MISMATCH, Shape, Side, binary_shape, names_dim_dimnames, set_ts, ts_late_error};
use crate::attrib::{self, Attrs};
use crate::kernel::logic as k;
use crate::kernel::{mismatch, result_len, zip2};
use crate::registry;
use crate::util::{is_number, logical_from_complex, not_yet};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Cx, Dispatched, Generic, Lgl, NeedGc, Out, RResult, Ref, Root, View};

/// `do_logic`: `&` (code 1), `|` (2) and `!` (3).
// R: logic.c:do_logic
pub fn do_logic<const CODE: u8>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let name = match CODE {
        1 => "&",
        2 => "|",
        _ => "!",
    };
    let argc = args.len();
    let attr = |cx: &Cx, i: usize| i < argc && attrib::has_attrib(args.get(cx, i));
    let attr1 = attr(cx, 0);
    let out = if attr1 || attr(cx, 1) {
        match cx.try_dispatch(&Ops::<CODE>::GENERIC, call, args, env, out)? {
            Dispatched::Done => return Ok(()),
            Dispatched::Default(out) => out,
        }
    } else {
        out
    };
    // checkArity(op, args): `&` and `|` take two, `!` one
    registry::by_name(name).expect("robigo: a logic operator in the registry").check_arity(cx, call, args)?;
    if args.any_package_altrep() {
        return Err(not_yet(cx, Some(call), "logic on a package ALTREP vector"));
    }
    if argc == 1 {
        let x = args.root(0);
        if !attr1
            && let View::Lgl(v) = x.bind(cx).view()
            && v.len() == 1
        {
            // `!` of a plain logical scalar
            let r = k::not(v.get(0).0);
            return cx.build(out, |cx| Ok(cx.alloc_copy(&[Lgl(r)])?.finish().into()));
        }
        return lunary(cx, call, x, out);
    }
    lbinary(cx, CODE, call, args.root(0), args.root(1), out)
}

/// `lbinary`: `&` and `|`.
// R: logic.c:lbinary
fn lbinary(cx: &mut Cx, code: u8, call: Root<'_>, x: Root<'_>, y: Root<'_>, out: Out<'_>) -> RResult<()> {
    let (raw2, nx, ny, shape, ts) = {
        let (xv, yv) = (x.bind(cx), y.bind(cx));
        let raw2 = xv.sexptype() == RAWSXP && yv.sexptype() == RAWSXP;
        let ok = |r: Ref<'_>| r.sexptype() == NILSXP || is_number(r);
        if !raw2 && !(ok(xv) && ok(yv)) {
            return Err(cx.error_call(Some(call), "operations are possible only for numeric, logical or complex types"));
        }
        let (nx, ny) = (xv.len().unwrap_or(0), yv.len().unwrap_or(0));
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
        (raw2, nx, ny, shape, ts)
    };
    let n = result_len(nx, ny);
    if mismatch(nx, ny) {
        cx.warning_call(Some(call), MISMATCH)?;
    }
    let late = ts_late_error(shape, None, ts, x.bind(cx), y.bind(cx), n, false);
    let ts = if late.is_some() { None } else { ts };
    cx.build(out, |cx| {
        let (xv, yv) = (x.bind(cx), y.bind(cx));
        let r = if raw2 {
            let (a, b) = (raws(xv), raws(yv));
            let mut v = cx.alloc_vector::<u8>(n)?;
            zip2(a, b, v.as_mut_slice(), |p, q| if code == 1 { p & q } else { p | q });
            finish(cx, v, shape, ts, xv, yv, n)?
        } else {
            let (a, b) = (logicals(xv), logicals(yv));
            let mut v = cx.alloc_vector::<Lgl>(n)?;
            zip2(&a, &b, v.as_mut_slice(), |p, q| Lgl(if code == 1 { k::and(p, q) } else { k::or(p, q) }));
            finish(cx, v, shape, ts, xv, yv, n)?
        };
        Ok(r)
    })?;
    match late {
        Some(msg) => Err(cx.error(msg)),
        None => Ok(()),
    }
}

fn finish<'g, T: robigo_rtcore::Elt<'g>>(
    cx: &'g Cx,
    mut v: robigo_rtcore::FreshVec<'g, T>,
    shape: Shape,
    ts: Option<Side>,
    x: Ref<'g>,
    y: Ref<'g>,
    n: usize,
) -> Result<Ref<'g>, NeedGc> {
    if attrib::has_attrib(x) || attrib::has_attrib(y) {
        let mut a = Attrs::new();
        binary_shape(cx, &mut a, shape, None, (true, true), true, x, y, n)?;
        if let Some(side) = ts {
            set_ts(cx, &mut a, if side == Side::X { x } else { y })?;
        }
        a.apply(cx, &mut v)?;
    }
    Ok(v.finish().into())
}

fn raws<'g>(x: Ref<'g>) -> &'g [u8] {
    x.as_vector::<u8>().and_then(|v| v.as_slice()).unwrap_or(&[])
}

/// An operand coerced to logical (`coerceVector(x, LGLSXP)`; NULL is
/// `logical(0)`).
// R: coerce.c:coerceToLogical
fn logicals<'g>(x: Ref<'g>) -> std::borrow::Cow<'g, [i32]> {
    use std::borrow::Cow;
    match x.view() {
        View::Lgl(_) => Cow::Borrowed(x.as_ints().unwrap()),
        View::Int(v) => Cow::Owned(v.as_slice().unwrap().iter().map(|&i| k::from_int(i)).collect()),
        View::Real(v) => Cow::Owned(v.as_slice().unwrap().iter().map(|&r| k::from_real(r)).collect()),
        View::Cplx(v) => Cow::Owned(v.as_slice().unwrap().iter().map(|&z| logical_from_complex(z)).collect()),
        _ => Cow::Borrowed(&[]),
    }
}

/// `lunary`: `!x`.
// R: logic.c:lunary
fn lunary(cx: &mut Cx, call: Root<'_>, x: Root<'_>, out: Out<'_>) -> RResult<()> {
    let xv = x.bind(cx);
    let len = xv.len().unwrap_or(0);
    let t = xv.sexptype();
    if !(t == LGLSXP || t == RAWSXP || is_number(xv)) {
        // GNU R reads `XLENGTH(R_NilValue)` as a vector's length word, which
        // there overlaps CAR(R_NilValue), R_NilValue itself: not 0, so
        // `!NULL` is the error
        if len == 0 && t != NILSXP {
            return cx.build(out, |cx| Ok(cx.alloc_vector::<Lgl>(0)?.finish().into()));
        }
        return Err(cx.error_call(Some(call), "invalid argument type"));
    }
    cx.build(out, |cx| {
        let xv = x.bind(cx);
        let mut a = Attrs::new();
        if matches!(xv.sexptype(), LGLSXP | RAWSXP) {
            // shallow_duplicate: every attribute
            a.copy_all(xv);
        } else {
            names_dim_dimnames(cx, &mut a, xv)?;
        }
        let r: Ref<'_> = match xv.view() {
            View::Raw(v) => {
                let s = v.as_slice().unwrap();
                let mut f = cx.alloc_with(s.len(), |i| 0xFF ^ s[i])?;
                a.apply(cx, &mut f)?;
                f.finish().into()
            }
            View::Real(v) => {
                let s = v.as_slice().unwrap();
                let mut f = cx.alloc_with(s.len(), |i| Lgl(k::not_real(s[i])))?;
                a.apply(cx, &mut f)?;
                f.finish().into()
            }
            View::Cplx(v) => {
                let s = v.as_slice().unwrap();
                let mut f = cx.alloc_with(s.len(), |i| {
                    let z = s[i];
                    Lgl(if z.r.is_nan() || z.i.is_nan() { k::NA } else { (z.r == 0.0 && z.i == 0.0) as i32 })
                })?;
                a.apply(cx, &mut f)?;
                f.finish().into()
            }
            _ => {
                let s = xv.as_ints().unwrap();
                let mut f = cx.alloc_with(s.len(), |i| Lgl(k::not(s[i])))?;
                a.apply(cx, &mut f)?;
                f.finish().into()
            }
        };
        Ok(r)
    })
}

/// The `Ops` group generic of `&` (code 1), `|` (2) or `!` (3).
struct Ops<const CODE: u8>;

impl<const CODE: u8> Ops<CODE> {
    const GENERIC: Generic = Generic::group(
        "Ops",
        match CODE {
            1 => "&",
            2 => "|",
            _ => "!",
        },
    );
}

/// `any(..., na.rm)` (`CODE` 2) and `all(..., na.rm)` (`CODE` 1): Summary
/// group dispatch, then each argument coerced to logical (with GNU R's
/// warning for a type other than integer).
// R: logic.c:do_logic3, checkValues; summary.c:fixup_NaRm
pub fn do_logic3<const CODE: i32>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    const ALL: i32 = 1;
    const ANY: i32 = 2;
    let name = if CODE == ALL { "all" } else { "any" };
    static GENERICS: [Generic; 2] = [Generic::group("Summary", "all"), Generic::group("Summary", "any")];
    // fixup_NaRm: na.rm last, FALSE if not given
    let n = args.len();
    let mut narm_at = None;
    for i in 0..n {
        if args.name(cx, i).is_some_and(|t| t.name().bytes() == b"na.rm") {
            if narm_at.is_some() {
                return Err(cx.error("formal argument \"na.rm\" matched by multiple actual arguments"));
            }
            narm_at = Some(i);
        }
    }
    cx.scope(|cx, s| {
        // the FALSE default and the na.rm symbol, made before the window
        let dflt = s.out(cx);
        let dflt = {
            let r = dflt.root();
            cx.build(dflt, |cx| Ok(cx.alloc_copy(&[Lgl(0)])?.finish().into()))?;
            r
        };
        let narm_sym = s.out(cx);
        let narm_sym = {
            let r = narm_sym.root();
            cx.build(narm_sym, |cx| Ok(cx.install(b"na.rm")?.as_ref()))?;
            r
        };
        let mut vals: Vec<robigo_rtcore::Ref<'_>> = Vec::with_capacity(n + 1);
        let mut names: Vec<Option<robigo_rtcore::Sym<'_>>> = Vec::with_capacity(n + 1);
        for i in 0..n {
            if Some(i) != narm_at {
                vals.push(args.get(cx, i));
                names.push(args.name(cx, i));
            }
        }
        vals.push(match narm_at {
            Some(i) => args.get(cx, i),
            None => dflt.bind(cx),
        });
        names.push(narm_sym.bind(cx).as_sym());
        let fixed = s.args(cx, &vals, Some(&names));
        let out = match cx.try_dispatch(&GENERICS[(CODE - 1) as usize], call, fixed, env, out)? {
            Dispatched::Done => return Ok(()),
            Dispatched::Default(o) => o,
        };
        let nf = fixed.len();
        let narm = crate::util::as_logical_n(cx, fixed.get(cx, nf - 1));
        if crate::util::is_vector_atomic(fixed.get(cx, nf - 1)) && narm.1 > 1 {
            let msg = format!("'length = {}' in coercion to 'logical(1)'", narm.1);
            return Err(cx.error_call(Some(call), &msg));
        }
        let narm = narm.0 == 1;
        let mut val = if CODE == ALL { 1 } else { 0 };
        let mut has_na = false;
        for i in 0..nf - 1 {
            let t = fixed.get(cx, i);
            let len = crate::array::r_length(cx, t);
            if len == 0 {
                continue;
            }
            let ty = t.sexptype();
            if ty != LGLSXP && ty != INTSXP {
                let msg = format!("coercing argument of type '{}' to logical", crate::util::type_to_char(t));
                cx.warning_call(Some(call), &msg)?;
            }
            let t = fixed.get(cx, i);
            let lv: Vec<i32> = match t.view() {
                robigo_rtcore::View::Lgl(v) => v.as_slice().unwrap().iter().map(|l| l.0).collect(),
                robigo_rtcore::View::Int(v) => v.as_slice().unwrap().iter().map(|&x| k::from_int(x)).collect(),
                robigo_rtcore::View::Real(v) => v.as_slice().unwrap().iter().map(|&x| k::from_real(x)).collect(),
                _ => {
                    let msg = format!(
                        "robigo: {name}() of a {} argument is not implemented yet",
                        crate::util::type_to_char(t)
                    );
                    return Err(cx.error_call(Some(call), &msg));
                }
            };
            // checkValues
            let mut na = false;
            let mut v = None;
            for &xi in &lv {
                if !narm && xi == i32::MIN {
                    na = true;
                } else if xi == 1 && CODE == ANY {
                    v = Some(1);
                    break;
                } else if xi == 0 && CODE == ALL {
                    v = Some(0);
                    break;
                }
            }
            let v = v.unwrap_or(if na {
                i32::MIN
            } else if CODE == ANY {
                0
            } else {
                1
            });
            val = v;
            if v != i32::MIN {
                if (CODE == ANY && v != 0) || (CODE == ALL && v == 0) {
                    has_na = false;
                    break;
                }
            } else {
                has_na = true;
            }
        }
        let r = if has_na { i32::MIN } else { val };
        cx.build(out, |cx| Ok(cx.alloc_copy(&[Lgl(r)])?.finish().into()))
    })
}
