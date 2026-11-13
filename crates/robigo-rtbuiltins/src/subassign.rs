//! subassign.c: the default methods of `[<-`, `[[<-` and `$<-`
//! (`do_subassign_dflt`, `do_subassign2_dflt`, `R_subassign3_dflt`),
//! shared by the VM's assignment specials, with GNU R's growth rule
//! (`EnlargeVector`, docs/g3.md §4) and its copy-on-modify protocol: the
//! object is written in place unless it may be shared (`MAYBE_SHARED`),
//! or referenced at all outside a complex assignment (`IS_ASSIGNMENT_CALL`).
//!
//! Vectors, lists and environments; matrix and array assignment
//! (`x[i, j] <- v`) and `[[<-` on a pairlist are not ported yet and say
//! so.

use crate::attrib::{self, Attr, Attrs};
use crate::coerce::coerce_vector;
use crate::subscript::{self, NA_IDX, NewName, Subscript, ecall, get1index, make_subscript, one_index};
use crate::subset::{install_sym, not_subsettable, vector_index};
use crate::util::{self, type_to_char};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Lgl, NeedGc, Out, RResult, Rcomplex, Ref, Root, Scope, View};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

const NA_INTEGER: i32 = i32::MIN;
const NA_REAL: f64 = crate::kernel::arith::NA_REAL;

/// GNU R's `R_EXPAND_FRAC` (read once): the over-allocation factor of
/// `EnlargeVector`, 1.05 unless the environment variable sets one in
/// [1, 2]. A value outside is the default, after the error "bad expand
/// value" the first time.
fn expand_frac() -> Result<f64, ()> {
    static EXPAND: OnceLock<(f64, bool)> = OnceLock::new();
    static REPORTED: AtomicBool = AtomicBool::new(false);
    let &(e, bad) = EXPAND.get_or_init(|| {
        let v = std::env::var("R_EXPAND_FRAC").ok().map(|s| atof(&s)).unwrap_or(1.05);
        if !(1.0..=2.0).contains(&v) { (1.05, true) } else { (v, false) }
    });
    if bad && !REPORTED.swap(true, Ordering::Relaxed) {
        return Err(());
    }
    Ok(e)
}

/// C's `atof`: the longest leading decimal number, 0 if none.
fn atof(s: &str) -> f64 {
    let t = s.trim_start();
    let end = t
        .char_indices()
        .take_while(|&(i, c)| {
            c.is_ascii_digit() || c == '.' || ((c == '-' || c == '+') && i == 0) || c == 'e' || c == 'E'
        })
        .map(|(i, c)| i + c.len_utf8())
        .last()
        .unwrap_or(0);
    (0..=end).rev().find_map(|k| t[..k].parse::<f64>().ok()).unwrap_or(0.0)
}

/// `xlength(x)` of a subassignment's operand: a vector's or pairlist's
/// length, an environment's number of bindings, 1 for anything else.
fn rlength(cx: &Cx, x: Ref<'_>) -> usize {
    match x.sexptype() {
        ENVSXP => match x.view() {
            View::Env(e) => cx.frame_bindings(e, true).map_or(0, |b| b.len()),
            _ => 0,
        },
        _ => x.len().unwrap_or(1),
    }
}

/// `getNames(x)`: the names attribute (a 1-d array's through `getAttrib`).
// R: subassign.c:getNames
fn get_names<'g>(x: Ref<'g>) -> Option<Ref<'g>> {
    if attrib::get(x, b"dim").is_some() { attrib::names(x) } else { attrib::get_value(x, b"names") }
}

/// The vector of `x`'s type with `x`'s `len` elements, then GNU R's fill up
/// to `cap`, growable at length `newlen` when `cap` exceeds it.
fn enlarged<'g>(cx: &'g Cx, x: Ref<'g>, newlen: usize, cap: usize, a: &Attrs<'g>) -> Result<Ref<'g>, NeedGc> {
    let len = x.len().unwrap_or(0);
    macro_rules! plain {
        ($v:expr, $na:expr) => {{
            let xs = $v.as_slice().unwrap();
            let mut r = cx.alloc_with(cap, |i| if i < len { xs[i] } else { $na })?;
            if newlen < cap {
                r.set_growable(newlen);
            }
            a.apply(cx, &mut r)?;
            r.finish().into()
        }};
    }
    Ok(match x.view() {
        View::Lgl(v) => plain!(v, Lgl::NA),
        View::Int(v) => plain!(v, NA_INTEGER),
        View::Real(v) => plain!(v, NA_REAL),
        View::Cplx(v) => plain!(v, Rcomplex { r: NA_REAL, i: NA_REAL }),
        View::Raw(v) => plain!(v, 0u8),
        View::Str(v) => {
            let xs = v.as_slice().unwrap();
            let mut r = cx.alloc_vector::<Char<'g>>(cap)?;
            let na = cx.na_string();
            for i in 0..cap {
                r.set(i, if i < len { xs[i] } else { na });
            }
            if newlen < cap {
                r.set_growable(newlen);
            }
            a.apply(cx, &mut r)?;
            r.finish().into()
        }
        View::List(v) | View::Expr(v) => {
            let xs = v.as_slice().unwrap();
            let mut r =
                if x.sexptype() == EXPRSXP { cx.alloc_expression(cap)? } else { cx.alloc_vector::<Ref<'g>>(cap)? };
            for (i, &e) in xs.iter().enumerate() {
                r.set(i, e);
            }
            if newlen < cap {
                r.set_growable(newlen);
            }
            a.apply(cx, &mut r)?;
            r.finish().into()
        }
        _ => unreachable!("robigo: EnlargeVector of a non-vector"),
    })
}

/// `EnlargeVector(x, newlen)`: the vector in root `x` lengthened to
/// `newlen` (the root then holds the result): in place when it is not
/// shared, growable and has room; else a new vector with 5% to spare
/// (`R_EXPAND_FRAC`), growable, its names enlarged with `""` and its other
/// attributes copied.
// R: subassign.c:EnlargeVector
pub fn enlarge_vector(cx: &mut Cx, x: Root<'_>, newlen: usize) -> RResult<()> {
    if !attrib::is_vector(x.bind(cx)) {
        return Err(cx.error("attempt to enlarge non-vector"));
    }
    if x.bind(cx).is_package_altrep() {
        return Err(cx.error("robigo: enlarging an ALTREP vector is not implemented yet"));
    }
    let len = x.bind(cx).len().unwrap_or(0);
    // options(check.bounds) is FALSE by default; options are B2b's
    cx.scope(|cx, s| {
        if cx.grow_in_place(x, newlen) {
            if attrib::has_attrib(x.bind(cx)) {
                if let Some(nm) = get_names(x.bind(cx)) {
                    let nr = s.root(cx, nm);
                    let o = s.out(cx);
                    let nn = o.root();
                    enlarge_names(cx, nr, len, newlen, o)?;
                    if nn.bind(cx) != nr.bind(cx) {
                        let sym = install_sym(cx, s, b"names")?;
                        cx.install_attrib(x, sym, nn)?;
                    }
                }
                for name in [&b"dim"[..], b"dimnames"] {
                    if attrib::get(x.bind(cx), name).is_some() {
                        let sym = install_sym(cx, s, name)?;
                        cx.remove_attrib(x, sym)?;
                    }
                }
            }
            return Ok(());
        }
        let expand = expand_frac().map_err(|_| cx.error("bad expand value"))?;
        let cap = if newlen > len {
            let e = newlen as f64 * expand;
            if e <= i64::MAX as f64 { e as usize } else { newlen }
        } else {
            newlen
        };
        let names = match get_names(x.bind(cx)) {
            Some(nm) => {
                let nr = s.root(cx, nm);
                let o = s.out(cx);
                let nn = o.root();
                enlarge_names(cx, nr, len, newlen, o)?;
                Some(nn)
            }
            None => None,
        };
        let o = s.out(cx);
        let r = o.root();
        cx.build(o, |cx| {
            let xv = x.bind(cx);
            let mut a = Attrs::new();
            if let Some(nn) = names {
                a.install(Attr { tag: cx.install(b"names")?, value: nn.bind(cx) });
            }
            a.copy_most(xv);
            enlarged(cx, xv, newlen, cap, &a)
        })?;
        x.replace_from(cx, r);
        Ok(())
    })
}

/// `EnlargeNames(names, len, newlen)`: the names enlarged as a vector, the
/// new ones `""`, written to `out`.
// R: subassign.c:EnlargeNames
fn enlarge_names(cx: &mut Cx, names: Root<'_>, len: usize, newlen: usize, out: Out<'_>) -> RResult<()> {
    let nv = names.bind(cx);
    if nv.sexptype() != STRSXP || nv.len() != Some(len) {
        return Err(cx.error("bad names attribute"));
    }
    cx.scope(|cx, s| {
        let nr = s.root(cx, names.bind(cx));
        enlarge_vector(cx, nr, newlen)?;
        {
            let mut u = cx.unique_list(nr)?;
            for i in len..newlen {
                u.set_blank(i);
            }
        }
        out.set(cx, nr.bind(cx));
        Ok(())
    })
}

/// `R_cycle_detected(s, child)`: whether `child` reaches `s` through
/// attributes, list elements or pairlist cells.
// R: duplicate.c:R_cycle_detected
fn cycle_detected(s: Ref<'_>, child: Ref<'_>) -> bool {
    if s == child {
        return !matches!(
            child.sexptype(),
            NILSXP | SYMSXP | ENVSXP | SPECIALSXP | BUILTINSXP | EXTPTRSXP | BCODESXP | WEAKREFSXP
        );
    }
    if let Some(a) = child.attrib()
        && a.sexptype() != NILSXP
        && cycle_detected(s, a)
    {
        return true;
    }
    match child.view() {
        View::Pairlist(_) | View::Lang(_) | View::Dots(_) => {
            let mut el = child.as_cons();
            while let Some(c) = el {
                if s == c.as_ref() || cycle_detected(s, c.car()) {
                    return true;
                }
                if let Some(a) = c.as_ref().attrib()
                    && a.sexptype() != NILSXP
                    && cycle_detected(s, a)
                {
                    return true;
                }
                el = c.cdr().as_cons();
            }
            false
        }
        View::List(v) | View::Expr(v) => (0..v.len()).any(|i| cycle_detected(s, v.get(i))),
        _ => false,
    }
}

/// `R_FixupRHS(x, y)`: the value in root `y`, duplicated if it contains
/// `x` (the root then holds the copy).
// R: attrib.c:R_FixupRHS
pub fn fixup_rhs(cx: &mut Cx, x: Root<'_>, y: Root<'_>) -> RResult<()> {
    let yv = y.bind(cx);
    if yv.sexptype() != NILSXP && yv.maybe_referenced() && cycle_detected(x.bind(cx), yv) {
        cx.scope(|cx, s| {
            let o = s.out(cx);
            let d = o.root();
            cx.duplicate(y, o)?;
            y.replace_from(cx, d);
            Ok(())
        })?;
    }
    Ok(())
}

/// `embedInVector(v, call)`: an S4 object as a one-element list (with a
/// deprecation warning); any other object is an error.
fn embed_in_vector(cx: &mut Cx, v: Root<'_>, call: Root<'_>) -> RResult<()> {
    if v.bind(cx).is_s4() {
        return Err(util::not_yet(cx, Some(call), "'implicit list embedding of S4 objects'"));
    }
    Err(cx.error_call(Some(call), "implicit list embedding of \"object\" is not possible"))
}

/// Replace the value in root `r` by its coercion to `ty`.
fn coerce_root(cx: &mut Cx, r: Root<'_>, ty: u8) -> RResult<()> {
    cx.scope(|cx, s| {
        let o = s.out(cx);
        let c = o.root();
        coerce_vector(cx, r, ty, o)?;
        r.replace_from(cx, c);
        Ok(())
    })
}

/// `SubassignTypeFix(&x, &y, stretch, level, call, rho)`: coerce the LHS
/// in root `x` (or the RHS in root `y`) so that one can take the other's
/// elements, then enlarge the LHS to `stretch`; returns `100 * TYPEOF(x) +
/// TYPEOF(y)` as they are now. Level 1 (`[<-`) coerces a non-list RHS into
/// a list; level 2 (`[[<-`) inserts it as it is.
// R: subassign.c:SubassignTypeFix
pub fn subassign_type_fix(
    cx: &mut Cx,
    x: Root<'_>,
    y: Root<'_>,
    stretch: usize,
    level: u8,
    call: Root<'_>,
) -> RResult<i32> {
    let (tx, ty) = (x.bind(cx).sexptype() as i32, y.bind(cx).sexptype() as i32);
    let which = 100 * tx + ty;
    let x_is_object = x.bind(cx).is_object();
    let mut redo = true;
    match which {
        1000 | 1300 | 1400 | 1500 | 1600 | 1900 | 2000 | 2400 | 1010 | 1310 | 1410 | 1510 | 1313 | 1413 | 1513
        | 1414 | 1514 | 1515 | 1616 | 1919 | 2020 | 2424 => redo = false,
        1013 => coerce_root(cx, x, INTSXP)?,
        1014 | 1314 => coerce_root(cx, x, REALSXP)?,
        1015 | 1315 | 1415 => coerce_root(cx, x, CPLXSXP)?,
        1610 | 1613 | 1614 | 1615 => coerce_root(cx, y, STRSXP)?,
        1016 | 1316 | 1416 | 1516 => coerce_root(cx, x, STRSXP)?,
        1901 | 1902 | 1904 | 1905 | 1906 | 1910 | 1913 | 1914 | 1915 | 1916 | 1920 | 1921 | 1922 | 1923 | 1924
        | 1903 | 1907 | 1908 | 1999 => {
            if level == 1 {
                coerce_root(cx, y, VECSXP)?;
            } else {
                redo = false;
            }
        }
        1925 | 2025 => {
            if level == 1 {
                embed_in_vector(cx, y, call)?;
            } else {
                redo = false;
            }
        }
        1019 | 1319 | 1419 | 1519 | 1619 | 2419 => coerce_root(cx, x, VECSXP)?,
        1020 | 1320 | 1420 | 1520 | 1620 | 2420 => coerce_root(cx, x, EXPRSXP)?,
        2001 | 2002 | 2006 | 2010 | 2013 | 2014 | 2015 | 2016 | 2019 => {
            if level == 1 {
                coerce_root(cx, y, VECSXP)?;
            } else {
                redo = false;
            }
        }
        1025 | 1325 | 1425 | 1525 | 1625 | 2425 => {
            return Err(util::not_yet(cx, Some(call), "'as.vector' dispatch on an object in subassignment"));
        }
        _ => {
            let msg = format!(
                "incompatible types (from {} to {}) in subassignment type fix",
                type_to_char(x.bind(cx)),
                type_to_char(y.bind(cx))
            );
            return Err(cx.error(&msg));
        }
    }
    if stretch > 0 {
        enlarge_vector(cx, x, stretch)?;
    }
    cx.set_object_bit(x, x_is_object);
    Ok(if redo { 100 * x.bind(cx).sexptype() as i32 + y.bind(cx).sexptype() as i32 } else { which })
}

/// `DeleteListElements(x, which)`: the list without the elements at the
/// positions, names and attributes kept (written to `out`; `x` itself if
/// none is removed).
// R: subassign.c:DeleteListElements
fn delete_list_elements(cx: &mut Cx, x: Root<'_>, which: &Subscript, out: Out<'_>) -> RResult<()> {
    let len = x.bind(cx).len().unwrap_or(0);
    let mut include = vec![true; len];
    for ii in which.iter() {
        if ii != NA_IDX && 0 < ii && ii as usize <= len {
            include[ii as usize - 1] = false;
        }
    }
    if include.iter().all(|&b| b) {
        out.set(cx, x.bind(cx));
        return Ok(());
    }
    cx.build(out, |cx| {
        let xv = x.bind(cx);
        let l = xv.as_vector::<Ref<'_>>().unwrap();
        let keep: Vec<usize> = (0..len).filter(|&i| include[i]).collect();
        let mut r = if xv.sexptype() == EXPRSXP {
            cx.alloc_expression(keep.len())?
        } else {
            cx.alloc_vector::<Ref<'_>>(keep.len())?
        };
        for (k, &i) in keep.iter().enumerate() {
            r.set(k, l.get(i));
        }
        let mut a = Attrs::new();
        if let Some(nm) = attrib::names(xv).and_then(|n| n.as_vector::<Char<'_>>()) {
            let mut n = cx.alloc_vector::<Char<'_>>(keep.len())?;
            for (k, &i) in keep.iter().enumerate() {
                n.set(k, nm.get(i));
            }
            a.install(Attr { tag: cx.install(b"names")?, value: n.finish().into() });
        }
        a.copy_most(xv);
        a.apply(cx, &mut r)?;
        Ok(r.finish().into())
    })
}

/// `CLEAR_VECTOR(x)`: a list's elements set to NULL (releasing their
/// counts) once it has been replaced by a new one.
fn clear_vector(cx: &mut Cx, x: Root<'_>) -> RResult<()> {
    if matches!(x.bind(cx).sexptype(), VECSXP | EXPRSXP) && !x.bind(cx).maybe_shared() {
        let mut u = cx.unique_list(x)?;
        for i in 0..u.len() {
            u.set_na(i);
        }
    }
    Ok(())
}

/// The `MOD_ITERATE1` loop of `VectorAssign`: `f(ii, iny)` for every
/// non-NA position `ii` (0-based) with the recycled RHS index `iny`.
fn assign_loop(idx: &Subscript, ny: usize, mut f: impl FnMut(usize, usize)) {
    for i in 0..idx.len() {
        let ii = idx.get(i);
        if ii == NA_IDX {
            continue;
        }
        f(ii as usize - 1, i % ny);
    }
}

/// `VectorAssign(call, rho, x, s, y)`: the vector in root `x` (made unique
/// by the caller) assigned `y` at the positions of the subscript `s`; the
/// root then holds the result.
// R: subassign.c:VectorAssign
fn vector_assign(cx: &mut Cx, call: Root<'_>, x: Root<'_>, s: Root<'_>, y: Root<'_>) -> RResult<()> {
    // the simple scalar case
    {
        let (xv, sv, yv) = (x.bind(cx), s.bind(cx), y.bind(cx));
        if !attrib::has_attrib(sv)
            && xv.sexptype() == REALSXP
            && !xv.is_package_altrep()
            && let Some(yd) = yv.as_vector::<f64>().filter(|v| v.len() == 1).map(|v| v.get(0))
        {
            let nx = xv.len().unwrap_or(0);
            let i = match sv.view() {
                View::Int(v) if v.len() == 1 && v.get(0) != NA_INTEGER => Some(v.get(0) as i64),
                View::Real(v) if v.len() == 1 && v.get(0).is_finite() => Some(v.get(0) as i64),
                _ => None,
            };
            if let Some(i) = i
                && 1 <= i
                && i as usize <= nx
            {
                cx.unique_mut::<f64>(x)?.as_mut_slice()[i as usize - 1] = yd;
                return Ok(());
            }
        }
    }
    if x.bind(cx).sexptype() == NILSXP && y.bind(cx).sexptype() == NILSXP {
        return Ok(());
    }
    {
        let (xv, sv) = (x.bind(cx), s.bind(cx));
        if attrib::has_attrib(sv)
            && let Some(sd) = attrib::dims(sv)
            && sd.len() == 2
            && let Some(xd) = attrib::dims(xv)
            && sd[1] as usize == xd.len()
        {
            return Err(util::not_yet(cx, Some(call), "'assignment by a matrix of subscripts'"));
        }
    }
    let idx = make_subscript(cx, x, s, true, None)?;
    let n = idx.len();
    if rlength(cx, y.bind(cx)) > 1 && idx.iter().any(|ii| ii == NA_IDX) {
        return Err(cx.error("NAs are not allowed in subscripted assignments"));
    }
    let which = subassign_type_fix(cx, x, y, idx.stretch, 1, call)?;
    if n == 0 {
        return Ok(());
    }
    let ny = rlength(cx, y.bind(cx));
    let nx = x.bind(cx).len().unwrap_or(0);
    let xt = x.bind(cx).sexptype();
    if !matches!(xt, VECSXP | EXPRSXP) || y.bind(cx).sexptype() != NILSXP {
        if ny == 0 {
            return Err(cx.error("replacement has length zero"));
        }
        if n % ny != 0 {
            cx.warning("number of items to replace is not a multiple of replacement length")?;
        }
    }
    cx.scope(|cx, sc| {
        // when array elements are permuted the RHS must be a copy
        let y = if x.bind(cx) == y.bind(cx) {
            let o = sc.out(cx);
            let d = o.root();
            cx.shallow_duplicate(y, o)?;
            d
        } else {
            y
        };
        match which {
            1010 => cx.edit_pair::<Lgl, Lgl, _>(x, y, |px, py| assign_loop(&idx, ny, |ii, iy| px[ii] = py[iy]))?,
            1310 => cx.edit_pair::<i32, Lgl, _>(x, y, |px, py| assign_loop(&idx, ny, |ii, iy| px[ii] = py[iy].0))?,
            1313 => cx.edit_pair::<i32, i32, _>(x, y, |px, py| assign_loop(&idx, ny, |ii, iy| px[ii] = py[iy]))?,
            1410 => cx.edit_pair::<f64, Lgl, _>(x, y, |px, py| {
                assign_loop(&idx, ny, |ii, iy| px[ii] = if py[iy].0 == NA_INTEGER { NA_REAL } else { py[iy].0 as f64 })
            })?,
            1413 => cx.edit_pair::<f64, i32, _>(x, y, |px, py| {
                assign_loop(&idx, ny, |ii, iy| px[ii] = if py[iy] == NA_INTEGER { NA_REAL } else { py[iy] as f64 })
            })?,
            1414 => cx.edit_pair::<f64, f64, _>(x, y, |px, py| assign_loop(&idx, ny, |ii, iy| px[ii] = py[iy]))?,
            1510 => cx.edit_pair::<Rcomplex, Lgl, _>(x, y, |px, py| {
                assign_loop(&idx, ny, |ii, iy| {
                    let v = py[iy].0;
                    px[ii] = Rcomplex { r: if v == NA_INTEGER { NA_REAL } else { v as f64 }, i: 0.0 }
                })
            })?,
            1513 => cx.edit_pair::<Rcomplex, i32, _>(x, y, |px, py| {
                assign_loop(&idx, ny, |ii, iy| {
                    let v = py[iy];
                    px[ii] = Rcomplex { r: if v == NA_INTEGER { NA_REAL } else { v as f64 }, i: 0.0 }
                })
            })?,
            1514 => cx.edit_pair::<Rcomplex, f64, _>(x, y, |px, py| {
                assign_loop(&idx, ny, |ii, iy| {
                    let v = py[iy];
                    px[ii] = Rcomplex { r: if robigo_text::is_na(v) { NA_REAL } else { v }, i: 0.0 }
                })
            })?,
            1515 => {
                cx.edit_pair::<Rcomplex, Rcomplex, _>(x, y, |px, py| assign_loop(&idx, ny, |ii, iy| px[ii] = py[iy]))?
            }
            2424 => cx.edit_pair::<u8, u8, _>(x, y, |px, py| assign_loop(&idx, ny, |ii, iy| px[ii] = py[iy]))?,
            1610 | 1613 | 1614 | 1615 | 1616 | 1919 | 2019 | 2020 => {
                let mut u = cx.unique_list(x)?;
                assign_loop(&idx, ny, |ii, iy| u.copy_from(ii, y, iy));
            }
            1900 | 2000 => {
                let o = sc.out(cx);
                let r = o.root();
                delete_list_elements(cx, x, &idx, o)?;
                x.replace_from(cx, r);
                return Ok(());
            }
            _ => cx.warning_call(Some(call), "sub assignment (*[*] <- *) not done; __bug?__")?,
        }
        // the names of the new elements (makeSubscript's use.names)
        if let Some(newnames) = &idx.use_names {
            let has = attrib::names(x.bind(cx)).is_some();
            let names = if has {
                sc.root(cx, attrib::names(x.bind(cx)).unwrap())
            } else {
                let o = sc.out(cx);
                let r = o.root();
                cx.build(o, |cx| Ok(cx.alloc_vector::<Char<'_>>(nx)?.finish().into()))?;
                r
            };
            {
                let mut u = cx.unique_list(names)?;
                for (i, nn) in newnames.iter().enumerate() {
                    let Some(j) = *nn else { continue };
                    let ii = idx.get(i);
                    if ii == NA_IDX {
                        continue;
                    }
                    u.copy_from(ii as usize - 1, s, j);
                }
            }
            if !has || attrib::names(x.bind(cx)) != Some(names.bind(cx)) {
                let sym = install_sym(cx, sc, b"names")?;
                cx.install_attrib(x, sym, names)?;
            }
        }
        Ok(())
    })
}

/// `PairToVectorList(x)` into root `x`.
fn pairs_to_list_root(cx: &mut Cx, x: Root<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let o = s.out(cx);
        let r = o.root();
        cx.build(o, |cx| crate::coerce::pair_to_vector_list(cx, x.bind(cx)))?;
        x.replace_from(cx, r);
        Ok(())
    })
}

/// `VectorToPairList(x)` then `SET_TYPEOF(x, LANGSXP)`: the list in root
/// `x` as a call, its names as tags.
fn list_to_lang_root(cx: &mut Cx, x: Root<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let o = s.out(cx);
        let r = o.root();
        cx.build(o, |cx| {
            let l = x.bind(cx);
            let v = l.as_vector::<Ref<'_>>().unwrap();
            let names = attrib::names(l).and_then(|n| n.as_vector::<Char<'_>>());
            let mut tail = cx.nil();
            for i in (0..v.len()).rev() {
                let tag = match names.map(|n| n.get(i)).filter(|c| !c.bytes().is_empty()) {
                    Some(n) => Some(cx.install(n.bytes())?),
                    None => None,
                };
                tail = match (i, tag) {
                    (0, Some(t)) => cx.lcons_tagged(v.get(i), tail, t)?,
                    (0, None) => cx.lcons(v.get(i), tail)?,
                    (_, Some(t)) => cx.cons_tagged(v.get(i), tail, t)?,
                    (_, None) => cx.cons(v.get(i), tail)?,
                };
            }
            Ok(tail)
        })?;
        x.replace_from(cx, r);
        Ok(())
    })
}

/// A copy of the object in root `x` that it alone holds: `x` ends holding
/// it (`SETCAR(args, shallow_duplicate(CAR(args)))`).
fn dup_root(cx: &mut Cx, s: Scope<'_>, x: Root<'_>) -> RResult<()> {
    let o = s.out(cx);
    let d = o.root();
    cx.shallow_duplicate(x, o)?;
    x.replace_from(cx, d);
    Ok(())
}

/// GNU R's rule before writing the object in root `x`: copy it if it may
/// be shared, or referenced at all outside a complex assignment.
fn make_local(cx: &mut Cx, s: Scope<'_>, call: Root<'_>, x: Root<'_>) -> RResult<()> {
    let xv = x.bind(cx);
    if xv.maybe_shared() || (!call.bind(cx).is_assignment_call() && xv.maybe_referenced()) {
        dup_root(cx, s, x)?;
    }
    Ok(())
}

/// `do_subassign_dflt`: `[<-`'s default method. The window holds `x`, the
/// subscripts and the value (last).
// R: subassign.c:do_subassign_dflt, SubAssignArgs
pub fn do_subassign_dflt(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let n = args.len();
    if n < 2 {
        return Err(cx.error("SubAssignArgs: invalid number of arguments"));
    }
    let nsubs = n - 2;
    cx.scope(|cx, s| {
        let x = s.root(cx, args.get(cx, 0));
        let y = s.root(cx, args.get(cx, n - 1));
        // the LHS must not be one of the indices (permute[permute, 1] <- 1:3)
        if (1..n - 1).any(|i| args.get(cx, i) == x.bind(cx)) {
            cx.mark_not_mutable(x);
        }
        make_local(cx, s, call, x)?;
        let s4 = x.bind(cx).is_s4();
        let oldtype = x.bind(cx).sexptype();
        if matches!(oldtype, LISTSXP | LANGSXP) {
            pairs_to_list_root(cx, x)?;
        } else if rlength(cx, x.bind(cx)) == 0 {
            let (xv, yv) = (x.bind(cx), y.bind(cx));
            if rlength(cx, yv) == 0
                && (xv.sexptype() == NILSXP
                    || xv.sexptype() == yv.sexptype()
                    || matches!(yv.sexptype(), VECSXP | EXPRSXP))
            {
                out.set(cx, xv);
                return Ok(());
            }
            if xv.sexptype() == NILSXP {
                let ty = yv.sexptype();
                coerce_root(cx, x, ty)?;
            }
        }
        match x.bind(cx).sexptype() {
            LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | EXPRSXP | VECSXP | RAWSXP => match nsubs {
                0 => {
                    let m = s.root(cx, cx.missing_arg());
                    vector_assign(cx, call, x, m, y)?
                }
                1 => vector_assign(cx, call, x, args.root(1), y)?,
                _ => return Err(util::not_yet(cx, Some(call), "'[<-' with matrix or array subscripts")),
            },
            _ => {
                let msg = not_subsettable(x.bind(cx));
                return Err(cx.error(&msg));
            }
        }
        if oldtype == LANGSXP {
            if x.bind(cx).len().unwrap_or(0) == 0 {
                return Err(cx.error("result is zero-length and so cannot be a language object"));
            }
            list_to_lang_root(cx, x)?;
        }
        if s4 {
            cx.set_s4_bit(x, true);
        }
        out.set(cx, x.bind(cx));
        Ok(())
    })
}

/// `DeleteOneVectorListItem(x, which)`: the list without element `which`
/// (written to `out`; `x` itself if out of range).
// R: subassign.c:DeleteOneVectorListItem
fn delete_one(cx: &mut Cx, x: Root<'_>, which: i64, out: Out<'_>) -> RResult<()> {
    let n = x.bind(cx).len().unwrap_or(0) as i64;
    if !(0 <= which && which < n) {
        out.set(cx, x.bind(cx));
        return Ok(());
    }
    delete_list_elements(cx, x, &Subscript::of(vec![which + 1]), out)
}

/// `do_subassign2_dflt`: `[[<-`'s default method. The window holds `x`,
/// the subscripts and the value (last).
// R: subassign.c:do_subassign2_dflt
pub fn do_subassign2_dflt(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let n = args.len();
    if n < 2 {
        return Err(cx.error("SubAssignArgs: invalid number of arguments"));
    }
    let nsubs = n - 2;
    cx.scope(|cx, s| {
        let x = s.root(cx, args.get(cx, 0));
        let y = s.root(cx, args.get(cx, n - 1));
        // NULL: NULL if the value is, else an empty list
        if x.bind(cx).sexptype() == NILSXP {
            if y.bind(cx).sexptype() == NILSXP {
                out.set(cx, cx.nil());
                return Ok(());
            }
            let o = s.out(cx);
            let r = o.root();
            cx.build(o, |cx| Ok(cx.alloc_vector::<Ref<'_>>(0)?.finish().into()))?;
            x.replace_from(cx, r);
        }
        make_local(cx, s, call, x)?;
        let s4 = x.bind(cx).is_s4();
        if s4 && x.bind(cx).sexptype() == OBJSXP {
            return Err(util::not_yet(cx, Some(call), "'[[<-' on an S4 object"));
        }
        // xtop: the object returned; xup: the container of x when recursing
        let xtop = s.root(cx, x.bind(cx));
        let xup = s.root(cx, x.bind(cx));
        let dims = attrib::dims(x.bind(cx)).map(|d| d.to_vec());
        if attrib::get_value(x.bind(cx), b"dim").is_some_and(|d| d.sexptype() != INTSXP) {
            return Err(cx.error("improper dimensions"));
        }
        let ndims = dims.as_ref().map_or(0, |d| d.len());
        if x.bind(cx).sexptype() == ENVSXP {
            let name = match args.get(cx, 1).as_vector::<Char<'_>>() {
                Some(v) if nsubs == 1 && v.len() == 1 => {
                    crate::text::to_native(crate::coerce::text(v.get(0)), cx.locale())
                        .map(|b| b.into_owned())
                        .unwrap_or_else(|_| v.get(0).bytes().to_vec())
                }
                _ => return Err(cx.error("wrong args for environment subassignment")),
            };
            let sym = install_sym(cx, s, &name)?;
            cx.define_var(sym, y, x)?;
            out.set(cx, x.bind(cx));
            return Ok(());
        }
        // a vector subscript for a list: recursive
        let thesub = if nsubs == 1 { Some(args.root(1)) } else { None };
        let len = thesub.map_or(0, |t| t.bind(cx).len().unwrap_or(0));
        let mut off = -1i64;
        let recursed = nsubs == 1 && len > 1;
        if recursed {
            let thesub = thesub.unwrap();
            vector_index(cx, xup, thesub, 0, len - 2, 1, call, true)?;
            let r = {
                let cx = &*cx;
                let xu = xup.bind(cx);
                one_index(cx, xu, thesub.bind(cx), xu.len().unwrap_or(0), Some(len - 2)).map(|(o, _)| o)
            };
            off = r.map_err(|m| ecall(cx, None, &m))?;
            x.replace_from(cx, xup);
            vector_index(cx, x, thesub, len - 2, len - 1, 1, call, true)?;
        }
        let xt = x.bind(cx).sexptype();
        if attrib::is_vector(x.bind(cx)) {
            let is_list = matches!(xt, VECSXP | EXPRSXP);
            let ylen = rlength(cx, y.bind(cx));
            if !is_list && ylen == 0 {
                return Err(cx.error("replacement has length zero"));
            }
            if !is_list && ylen > 1 {
                return Err(cx.error("more elements supplied than there are to replace"));
            }
            if nsubs == 0 || args.get(cx, 1) == cx.missing_arg() {
                return Err(cx.error(subscript::MISSING_SUBSCRIPT));
            }
            let mut stretch = 0usize;
            let offset;
            let mut newname: Option<Root<'_>> = None;
            if nsubs == 1 {
                let r = {
                    let cx = &*cx;
                    let xv = x.bind(cx);
                    let pos = if recursed { Some(len - 1) } else { None };
                    one_index(cx, xv, args.get(cx, 1), xv.len().unwrap_or(0), pos).map(|(o, nn)| {
                        (
                            o,
                            match nn {
                                NewName::Char(c) => Some(c.as_ref()),
                                NewName::None => None,
                            },
                        )
                    })
                };
                let (o, nn) = match r {
                    Ok((o, nn)) => (o, nn.map(|c| s.root(cx, c))),
                    Err(m) => return Err(ecall(cx, None, &m)),
                };
                newname = nn;
                if is_list && y.bind(cx).sexptype() == NILSXP {
                    let o2 = s.out(cx);
                    let r = o2.root();
                    delete_one(cx, x, o, o2)?;
                    if recursed {
                        if matches!(xup.bind(cx).sexptype(), VECSXP | EXPRSXP) {
                            cx.unique_list(xup)?.set(off as usize, r);
                        } else {
                            return Err(util::not_yet(cx, Some(call), "'[[<-' into a pairlist element"));
                        }
                    } else {
                        if r.bind(cx) != x.bind(cx) {
                            clear_vector(cx, x)?;
                        }
                        xtop.replace_from(cx, r);
                    }
                    out.set(cx, xtop.bind(cx));
                    return Ok(());
                }
                if o < 0 {
                    return Err(cx.error(&format!("[[ ]] {}", subscript::OUT_OF_BOUNDS)));
                }
                if o as usize >= x.bind(cx).len().unwrap_or(0) {
                    stretch = o as usize + 1;
                }
                offset = o as usize;
            } else {
                if ndims != nsubs {
                    return Err(cx.error("[[ ]] improper number of subscripts"));
                }
                let dims = dims.unwrap();
                let mut pos = Vec::with_capacity(ndims);
                for (i, &d) in dims.iter().enumerate() {
                    let r = {
                        let cx = &*cx;
                        let xv = x.bind(cx);
                        let dn = attrib::get_value(xv, b"dimnames").and_then(|d| d.as_vector::<Ref<'_>>());
                        let names = match dn {
                            Some(l) => subscript::Names::of_vector(l.get(i)),
                            None => subscript::Names::None,
                        };
                        get1index(cx, args.get(cx, 1 + i), &names, d as usize, 0, None).map(|(o, _)| o)
                    };
                    let p = r.map_err(|m| ecall(cx, Some(call), &m))?;
                    if p < 0 || p >= d as i64 {
                        return Err(cx.error(&format!("[[ ]] {}", subscript::OUT_OF_BOUNDS)));
                    }
                    pos.push(p);
                }
                let mut o = 0i64;
                for i in (1..ndims).rev() {
                    o = (o + pos[i]) * dims[i - 1] as i64;
                }
                offset = (o + pos[0]) as usize;
            }
            let old_x = s.root(cx, x.bind(cx));
            let which = subassign_type_fix(cx, x, y, stretch, 2, call)?;
            match which {
                1010 => cx.edit_pair::<Lgl, Lgl, _>(x, y, |px, py| px[offset] = py[0])?,
                1310 => cx.edit_pair::<i32, Lgl, _>(x, y, |px, py| px[offset] = py[0].0)?,
                1313 => cx.edit_pair::<i32, i32, _>(x, y, |px, py| px[offset] = py[0])?,
                1410 => cx.edit_pair::<f64, Lgl, _>(x, y, |px, py| {
                    px[offset] = if py[0].0 == NA_INTEGER { NA_REAL } else { py[0].0 as f64 }
                })?,
                1413 => cx.edit_pair::<f64, i32, _>(x, y, |px, py| {
                    px[offset] = if py[0] == NA_INTEGER { NA_REAL } else { py[0] as f64 }
                })?,
                1414 => cx.edit_pair::<f64, f64, _>(x, y, |px, py| px[offset] = py[0])?,
                1510 => cx.edit_pair::<Rcomplex, Lgl, _>(x, y, |px, py| {
                    let v = py[0].0;
                    px[offset] = Rcomplex { r: if v == NA_INTEGER { NA_REAL } else { v as f64 }, i: 0.0 }
                })?,
                1513 => cx.edit_pair::<Rcomplex, i32, _>(x, y, |px, py| {
                    let v = py[0];
                    px[offset] = Rcomplex { r: if v == NA_INTEGER { NA_REAL } else { v as f64 }, i: 0.0 }
                })?,
                1514 => cx.edit_pair::<Rcomplex, f64, _>(x, y, |px, py| {
                    let v = py[0];
                    px[offset] = Rcomplex { r: if robigo_text::is_na(v) { NA_REAL } else { v }, i: 0.0 }
                })?,
                1515 => cx.edit_pair::<Rcomplex, Rcomplex, _>(x, y, |px, py| px[offset] = py[0])?,
                1610 | 1613 | 1614 | 1615 | 1616 => cx.unique_list(x)?.copy_from(offset, y, 0),
                1019 | 1319 | 1419 | 1519 | 1619 | 1901 | 1902 | 1904 | 1905 | 1906 | 1910 | 1913 | 1914 | 1915
                | 1916 | 1920 | 1921 | 1922 | 1923 | 1924 | 1925 | 1903 | 1907 | 1908 | 1999 | 2001 | 2002 | 2006
                | 2010 | 2013 | 2014 | 2015 | 2016 | 2024 | 2025 | 1919 | 2020 => {
                    let cur = x.bind(cx).as_vector::<Ref<'_>>().unwrap().get(offset);
                    if y.bind(cx).maybe_referenced() && cur != y.bind(cx) {
                        fixup_rhs(cx, x, y)?;
                    }
                    cx.unique_list(x)?.set(offset, y);
                }
                2424 => cx.edit_pair::<u8, u8, _>(x, y, |px, py| px[offset] = py[0])?,
                _ => {
                    let msg = format!(
                        "incompatible types (from {} to {}) in [[ assignment",
                        type_to_char(x.bind(cx)),
                        type_to_char(y.bind(cx))
                    );
                    return Err(cx.error(&msg));
                }
            }
            // a stretch may bring a new name
            if stretch > 0
                && let Some(nn) = newname
            {
                match attrib::get_value(x.bind(cx), b"names") {
                    None => {
                        let o = s.out(cx);
                        let names = o.root();
                        let nx = x.bind(cx).len().unwrap_or(0);
                        cx.build(o, |cx| {
                            let mut v = cx.alloc_vector::<Char<'_>>(nx)?;
                            v.set(offset, nn.bind(cx).as_char().unwrap());
                            Ok(v.finish().into())
                        })?;
                        let sym = install_sym(cx, s, b"names")?;
                        cx.install_attrib(x, sym, names)?;
                    }
                    Some(names) => {
                        let names = s.root(cx, names);
                        let ch = s.root(cx, nn.bind(cx));
                        let wrapped = {
                            let o = s.out(cx);
                            let r = o.root();
                            cx.build(o, |cx| {
                                let mut v = cx.alloc_vector::<Char<'_>>(1)?;
                                v.set(0, ch.bind(cx).as_char().unwrap());
                                Ok(v.finish().into())
                            })?;
                            r
                        };
                        cx.unique_list(names)?.copy_from(offset, wrapped, 0);
                        if attrib::get_value(x.bind(cx), b"names") != Some(names.bind(cx)) {
                            let sym = install_sym(cx, s, b"names")?;
                            cx.install_attrib(x, sym, names)?;
                        }
                    }
                }
            }
            if old_x.bind(cx) != x.bind(cx) && !recursed {
                clear_vector(cx, old_x)?;
            }
        } else if matches!(xt, LISTSXP | LANGSXP) {
            return Err(util::not_yet(cx, Some(call), "'[[<-' on a pairlist"));
        } else {
            let msg = not_subsettable(x.bind(cx));
            return Err(cx.error(&msg));
        }
        if recursed {
            if matches!(xup.bind(cx).sexptype(), VECSXP | EXPRSXP) {
                cx.unique_list(xup)?.set(off as usize, x);
            } else {
                return Err(util::not_yet(cx, Some(call), "'[[<-' into a pairlist element"));
            }
            if len == 2 {
                xtop.replace_from(cx, xup);
            }
        } else {
            xtop.replace_from(cx, x);
        }
        if s4 {
            cx.set_s4_bit(xtop, true);
        }
        out.set(cx, xtop.bind(cx));
        Ok(())
    })
}

/// `R_subassign3_dflt(call, x, nlist, val)`: `$<-`'s default method; the
/// name is the symbol in root `nlist`.
// R: subassign.c:R_subassign3_dflt
pub fn subassign3_dflt(
    cx: &mut Cx,
    call: Root<'_>,
    x: Root<'_>,
    nlist: Root<'_>,
    val: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    cx.scope(|cx, s| {
        let x = s.root(cx, x.bind(cx));
        let val = s.root(cx, val.bind(cx));
        let s4 = x.bind(cx).is_s4();
        make_local(cx, s, call, x)?;
        let t = x.bind(cx).sexptype();
        if t == OBJSXP {
            return Err(util::not_yet(cx, Some(call), "'$<-' on an S4 object"));
        }
        let name = nlist.bind(cx).as_sym().expect("robigo: '$<-' takes a symbol").name().bytes().to_vec();
        if matches!(t, LISTSXP | LANGSXP) {
            pairlist_assign(cx, s, x, nlist, val)?;
        } else if t == NILSXP {
            if val.bind(cx).sexptype() != NILSXP {
                let o = s.out(cx);
                let r = o.root();
                cx.build(o, |cx| {
                    let sym = nlist.bind(cx).as_sym().unwrap();
                    cx.cons_tagged(val.bind(cx), cx.nil(), sym)
                })?;
                x.replace_from(cx, r);
            }
        } else if t == ENVSXP {
            cx.define_var(nlist, val, x)?;
        } else if matches!(t, SYMSXP | CLOSXP | SPECIALSXP | BUILTINSXP) {
            let msg = not_subsettable(x.bind(cx));
            return Err(cx.error(&msg));
        } else {
            let mut ty = VECSXP;
            if t == EXPRSXP {
                ty = EXPRSXP;
            } else if t != VECSXP {
                cx.warning("Coercing LHS to a list")?;
                coerce_root(cx, x, VECSXP)?;
            }
            let (imatch, nx) = {
                let cx = &*cx;
                let xv = x.bind(cx);
                let names = attrib::names(xv).and_then(|n| n.as_vector::<Char<'_>>());
                let nx = xv.len().unwrap_or(0);
                let im = names.and_then(|nm| {
                    (0..nx).find(|&i| {
                        let c = nm.get(i);
                        c != cx.na_string() && !c.bytes().is_empty() && {
                            let a = crate::coerce::text(c);
                            crate::text::seql(a, robigo_text::Str::new(&name, robigo_text::Enc::Native), cx.locale())
                        }
                    })
                });
                (im, nx)
            };
            if val.bind(cx).sexptype() == NILSXP {
                // an element deletion if the name matches
                if let Some(im) = imatch {
                    let o = s.out(cx);
                    let r = o.root();
                    delete_list_elements(cx, x, &Subscript::of(vec![im as i64 + 1]), o)?;
                    clear_vector(cx, x)?;
                    x.replace_from(cx, r);
                }
            } else if let Some(im) = imatch {
                let cur = x.bind(cx).as_vector::<Ref<'_>>().unwrap().get(im);
                if val.bind(cx).maybe_referenced() && cur != val.bind(cx) {
                    fixup_rhs(cx, x, val)?;
                }
                cx.unique_list(x)?.set(im, val);
            } else {
                // a new element: a new list, one longer
                let o = s.out(cx);
                let r = o.root();
                cx.build(o, |cx| {
                    let xv = x.bind(cx);
                    let l = xv.as_vector::<Ref<'_>>().unwrap();
                    let mut v =
                        if ty == EXPRSXP { cx.alloc_expression(nx + 1)? } else { cx.alloc_vector::<Ref<'_>>(nx + 1)? };
                    for i in 0..nx {
                        v.set(i, l.get(i));
                    }
                    v.set(nx, val.bind(cx));
                    let mut nm = cx.alloc_vector::<Char<'_>>(nx + 1)?;
                    if let Some(old) = attrib::names(xv).and_then(|n| n.as_vector::<Char<'_>>()) {
                        for i in 0..nx {
                            nm.set(i, old.get(i));
                        }
                    }
                    nm.set(nx, nlist.bind(cx).as_sym().unwrap().name());
                    let mut a = Attrs::new();
                    a.install(Attr { tag: cx.install(b"names")?, value: nm.finish().into() });
                    a.copy_most(xv);
                    a.apply(cx, &mut v)?;
                    Ok(v.finish().into())
                })?;
                clear_vector(cx, x)?;
                x.replace_from(cx, r);
            }
        }
        if s4 {
            cx.set_s4_bit(x, true);
        }
        out.set(cx, x.bind(cx));
        Ok(())
    })
}

/// `R_subassign3_dflt`'s pairlist case: the cell tagged `nlist` replaced
/// (removed for NULL), else a new cell appended.
fn pairlist_assign(cx: &mut Cx, s: Scope<'_>, x: Root<'_>, nlist: Root<'_>, val: Root<'_>) -> RResult<()> {
    let is_null = val.bind(cx).sexptype() == NILSXP;
    let (head_match, head_car_is_val) = {
        let head = x.bind(cx).as_cons().unwrap();
        (head.tag() == nlist.bind(cx), head.car() == val.bind(cx))
    };
    if head_match {
        if is_null {
            // the rest of the list takes the head's attributes and bits
            return Err(util::not_yet(cx, None, "'$<- NULL' on the first element of a pairlist"));
        }
        if val.bind(cx).maybe_referenced() && !head_car_is_val {
            fixup_rhs(cx, x, val)?;
        }
        cx.set_car(x, val);
        return Ok(());
    }
    let cur = s.root(cx, x.bind(cx));
    loop {
        // the next cell, whether it is the one, and its car is the value
        let (next, hit, same) = {
            let c = cur.bind(cx).as_cons().unwrap();
            let next = c.cdr();
            let (hit, same) = match next.as_cons() {
                Some(nc) => (nc.tag() == nlist.bind(cx), nc.car() == val.bind(cx)),
                None => (false, false),
            };
            (s.root(cx, next), hit, same)
        };
        if hit {
            if is_null {
                let nil = s.root(cx, cx.nil());
                cx.set_car(next, nil);
                let after = s.root(cx, next.bind(cx).as_cons().unwrap().cdr());
                cx.set_cdr(cur, after);
            } else {
                if val.bind(cx).maybe_referenced() && !same {
                    fixup_rhs(cx, x, val)?;
                }
                cx.set_car(next, val);
            }
            return Ok(());
        }
        if next.bind(cx).sexptype() == NILSXP {
            if !is_null {
                let o = s.out(cx);
                let cell = o.root();
                cx.build(o, |cx| cx.cons_tagged(val.bind(cx), cx.nil(), nlist.bind(cx).as_sym().unwrap()))?;
                cx.set_cdr(cur, cell);
            }
            return Ok(());
        }
        cur.replace_from(cx, next);
    }
}
