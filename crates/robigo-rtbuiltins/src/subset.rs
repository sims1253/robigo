//! subset.c: the default methods of `[`, `[[` and `$` (`do_subset_dflt`,
//! `do_subset2_dflt`, `R_subset3_dflt`), shared by the builtins `.subset`
//! and `.subset2` and the VM's subsetting specials (which evaluate the
//! arguments and try dispatch first, `R_DispatchOrEvalSP`).
//!
//! Vectors, lists, pairlists, calls and environments; matrix and array
//! subscripts (`x[i, j]`, `x[m]` with a matrix `m`) are not ported yet and
//! say so.

use crate::attrib::{self, Attr, Attrs};
use crate::subscript::{self, Names, OUT_OF_BOUNDS, Subscript, ecall, get1index, make_subscript};
use crate::util::{self, type_to_char};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Lgl, NeedGc, Out, RResult, Rcomplex, Ref, Root, Scope, View};

/// GNU R's `NA_REAL`.
const NA_REAL: f64 = crate::kernel::arith::NA_REAL;
const NA_INTEGER: i32 = i32::MIN;

/// GNU R's `R_MSG_ob_nonsub`.
pub fn not_subsettable(x: Ref<'_>) -> String {
    format!("object of type '{}' is not subsettable", type_to_char(x))
}

/// The arguments of a subsetting call after `ExtractArg` took out `drop`
/// and `exact` (the first argument so named, anywhere): the window
/// positions that remain, in order.
pub struct SubArgs {
    pub pos: Vec<usize>,
    pub drop: Option<usize>,
    pub exact: Option<usize>,
}

impl SubArgs {
    /// `ExtractArg(args, R_DropSymbol)` if `drop`, then
    /// `ExtractArg(args, R_ExactSymbol)` if `exact`.
    // R: subset.c:ExtractArg
    pub fn of(cx: &Cx, args: Args<'_>, drop: bool, exact: bool) -> SubArgs {
        let mut pos: Vec<usize> = (0..args.len()).collect();
        let mut take = |name: &[u8]| {
            let i = pos.iter().position(|&i| args.name(cx, i).is_some_and(|s| s.name().bytes() == name))?;
            Some(pos.remove(i))
        };
        let drop = if drop { take(b"drop") } else { None };
        let exact = if exact { take(b"exact") } else { None };
        SubArgs { pos, drop, exact }
    }
}

/// `ExtractDropArg`: `asLogical` of the `drop` argument, NA (or none) TRUE.
// R: subset.c:ExtractDropArg
fn drop_arg(cx: &Cx, args: Args<'_>, a: &SubArgs) -> bool {
    match a.drop {
        None => true,
        Some(i) => {
            let d = util::as_logical(cx, args.get(cx, i));
            d == NA_INTEGER || d != 0
        }
    }
}

/// `ExtractExactArg`: 1 exact (the default), 0 partial, -1 partial with a
/// warning (NA).
// R: subset.c:ExtractExactArg
fn exact_arg(cx: &Cx, args: Args<'_>, a: &SubArgs) -> i32 {
    match a.exact {
        None => 1,
        Some(i) => {
            let v = args.get(cx, i);
            if v.sexptype() == NILSXP {
                return 1;
            }
            let e = util::as_logical(cx, v);
            if e == NA_INTEGER { -1 } else { e }
        }
    }
}

/// `scalarIndex(s)`: a scalar integer or double without attributes as a
/// position (-1 for anything else, NA or not finite).
// R: subset.c:scalarIndex
fn scalar_index(s: Ref<'_>) -> i64 {
    if attrib::has_attrib(s) {
        return -1;
    }
    match s.view() {
        View::Int(v) if v.len() == 1 => {
            let i = v.get(0);
            if i == NA_INTEGER { -1 } else { i as i64 }
        }
        View::Real(v) if v.len() == 1 => {
            let d = v.get(0);
            if d.is_finite() { d as i64 } else { -1 }
        }
        _ => -1,
    }
}

/// A scalar of element `i` of the atomic vector `x` (`ScalarReal(REAL_ELT(x,
/// i))` and the others), or None for another type.
fn scalar_of<'g>(cx: &'g Cx, x: Ref<'g>, i: usize) -> Option<Result<Ref<'g>, NeedGc>> {
    Some(match x.view() {
        View::Real(v) => cx.alloc_copy(&[v.get(i)]).map(|v| v.finish().into()),
        View::Int(v) => cx.alloc_copy(&[v.get(i)]).map(|v| v.finish().into()),
        View::Lgl(v) => cx.alloc_copy(&[v.get(i)]).map(|v| v.finish().into()),
        View::Cplx(v) => cx.alloc_copy(&[v.get(i)]).map(|v| v.finish().into()),
        View::Raw(v) => cx.alloc_copy(&[v.get(i)]).map(|v| v.finish().into()),
        _ => return None,
    })
}

/// `ExtractSubset(x, indx, call)`: a new vector of `x`'s type with the
/// elements at the positions (NA, `NA_STRING`, R_NilValue or 0 for a
/// position out of range or NA). No attributes.
// R: subset.c:ExtractSubset
pub fn extract_subset<'g>(cx: &'g Cx, x: Ref<'g>, idx: &Subscript) -> Result<Ref<'g>, NeedGc> {
    let n = idx.len();
    let nx = x.len().unwrap_or(0) as i64;
    // the 0-based position of index i, or None
    let at = |i: usize| {
        let ii = idx.get(i);
        (ii != subscript::NA_IDX && 0 < ii && ii <= nx).then(|| (ii - 1) as usize)
    };
    macro_rules! plain {
        ($v:expr, $na:expr) => {{
            let xs = $v.as_slice().unwrap();
            cx.alloc_with(n, |i| at(i).map_or($na, |j| xs[j])).map(|v| v.finish().into())
        }};
    }
    match x.view() {
        View::Null => Ok(x),
        View::Lgl(v) => plain!(v, Lgl::NA),
        View::Int(v) => plain!(v, NA_INTEGER),
        View::Real(v) => plain!(v, NA_REAL),
        View::Cplx(v) => plain!(v, Rcomplex { r: NA_REAL, i: NA_REAL }),
        View::Raw(v) => plain!(v, 0u8),
        View::Str(v) => {
            let xs = v.as_slice().unwrap();
            let mut r = cx.alloc_vector::<Char<'g>>(n)?;
            let na = cx.na_string();
            for i in 0..n {
                r.set(i, at(i).map_or(na, |j| xs[j]));
            }
            Ok(r.finish().into())
        }
        View::List(v) | View::Expr(v) => {
            let xs = v.as_slice().unwrap();
            let mut r = if x.sexptype() == EXPRSXP { cx.alloc_expression(n)? } else { cx.alloc_vector::<Ref<'g>>(n)? };
            for i in 0..n {
                if let Some(j) = at(i) {
                    r.set(i, xs[j]);
                }
            }
            Ok(r.finish().into())
        }
        _ => unreachable!("robigo: ExtractSubset of a non-vector"),
    }
}

/// `VectorSubset(x, s, call)` into `out`: the subset's elements, its names
/// (and a list's `srcref`) subset alike. `x` is a vector or NULL.
// R: subset.c:VectorSubset
fn vector_subset(cx: &mut Cx, x: Root<'_>, s: Root<'_>, call: Root<'_>, out: Out<'_>) -> RResult<()> {
    if s.bind(cx) == cx.missing_arg() {
        return cx.duplicate(x, out);
    }
    {
        let (xv, sv) = (x.bind(cx), s.bind(cx));
        if attrib::has_attrib(sv)
            && let Some(sd) = attrib::dims(sv)
            && sd.len() == 2
            && let Some(xd) = attrib::dims(xv)
            && sd[1] as usize == xd.len()
        {
            return Err(util::not_yet(cx, Some(call), "'subsetting by a matrix of subscripts'"));
        }
    }
    let idx = make_subscript(cx, x, s, true, Some(call))?;
    cx.build(out, |cx| {
        let xv = x.bind(cx);
        let r = extract_subset(cx, xv, &idx)?;
        if r.sexptype() == NILSXP {
            return Ok(r);
        }
        let mut a = Attrs::new();
        let names = attrib::names(xv).or_else(|| {
            // a 1-d array's row names
            (attrib::is_array(xv) && attrib::len(attrib::get_value(xv, b"dimnames")) == 1)
                .then(|| attrib::get_value(xv, b"dimnames"))
                .flatten()
                .and_then(|dn| dn.as_vector::<Ref<'_>>().map(|l| l.get(0)))
                .filter(|n| n.sexptype() != NILSXP)
        });
        if let Some(nm) = names {
            let tag = cx.install(b"names")?;
            a.install(Attr { tag, value: extract_subset(cx, nm, &idx)? });
        }
        if let Some(sr) = attrib::get(xv, b"srcref").filter(|a| a.value.sexptype() == VECSXP) {
            a.install(Attr { tag: sr.tag, value: extract_subset(cx, sr.value, &idx)? });
        }
        set_attrs(cx, r, &a)
    })
}

/// A fresh vector `r` with the attributes `a` (made under the same `&Cx`
/// interval).
fn set_attrs<'g>(cx: &'g Cx, r: Ref<'g>, a: &Attrs<'g>) -> Result<Ref<'g>, NeedGc> {
    if a.is_empty() {
        return Ok(r);
    }
    macro_rules! with {
        ($t:ty) => {{
            let mut v = cx.alloc_copy(r.as_vector::<$t>().unwrap().as_slice().unwrap())?;
            a.apply(cx, &mut v)?;
            Ok(v.finish().into())
        }};
    }
    // the fresh result is re-made with its attributes: a builder applies
    // them to an unpublished vector
    match r.view() {
        View::Lgl(_) => with!(Lgl),
        View::Int(_) => with!(i32),
        View::Real(_) => with!(f64),
        View::Cplx(_) => with!(Rcomplex),
        View::Raw(_) => with!(u8),
        View::Str(v) => {
            let xs = v.as_slice().unwrap();
            let mut n = cx.alloc_vector::<Char<'g>>(xs.len())?;
            for (i, &c) in xs.iter().enumerate() {
                n.set(i, c);
            }
            a.apply(cx, &mut n)?;
            Ok(n.finish().into())
        }
        View::List(v) | View::Expr(v) => {
            let xs = v.as_slice().unwrap();
            let mut n = if r.sexptype() == EXPRSXP {
                cx.alloc_expression(xs.len())?
            } else {
                cx.alloc_vector::<Ref<'g>>(xs.len())?
            };
            for (i, &e) in xs.iter().enumerate() {
                n.set(i, e);
            }
            a.apply(cx, &mut n)?;
            Ok(n.finish().into())
        }
        _ => unreachable!("robigo: attributes for a non-vector result"),
    }
}

/// A pairlist's or call's elements as a list with its tags as names
/// (`do_subset_dflt`'s coercion of pair-based objects).
fn pairs_to_list<'g>(cx: &'g Cx, x: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
    let mut items = Vec::new();
    let mut tags = Vec::new();
    let mut any = false;
    let mut c = x.as_cons();
    while let Some(cell) = c {
        items.push(cell.car());
        let t = cell.tag().as_sym().map(|s| s.name());
        any |= t.is_some();
        tags.push(t);
        c = cell.cdr().as_cons();
    }
    let mut v = cx.alloc_vector::<Ref<'g>>(items.len())?;
    for (i, &e) in items.iter().enumerate() {
        v.set(i, e);
    }
    if any {
        let mut nm = cx.alloc_vector::<Char<'g>>(tags.len())?;
        for (i, t) in tags.iter().enumerate() {
            nm.set(i, t.unwrap_or_else(|| cx.blank_string()));
        }
        let mut a = Attrs::new();
        a.install(Attr { tag: cx.install(b"names")?, value: nm.finish().into() });
        a.apply(cx, &mut v)?;
    }
    Ok(v.finish().into())
}

/// A list as a call (`do_subset_dflt`'s LANGSXP result): its elements, its
/// names as tags.
fn list_to_call<'g>(cx: &'g Cx, l: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
    let v = l.as_vector::<Ref<'g>>().unwrap();
    let names = attrib::names(l).and_then(|n| n.as_vector::<Char<'g>>());
    let na = cx.na_string();
    let mut tail = cx.nil();
    for i in (0..v.len()).rev() {
        let tag = match names.map(|n| n.get(i)).filter(|c| !c.bytes().is_empty() && *c != na) {
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
}

/// The symbol `name`, after a collection if the table needs room.
pub fn install_sym<'s>(cx: &mut Cx, s: Scope<'s>, name: &[u8]) -> RResult<Root<'s>> {
    loop {
        match cx.install(name) {
            Ok(sym) => return Ok(s.root(cx, sym.as_ref())),
            Err(e) => cx.need_gc(e)?,
        }
    }
}

/// Whether `[`'s scalar fast paths take `x`: an atomic vector of these
/// types in standard storage.
fn scalar_type(x: Ref<'_>) -> bool {
    matches!(x.sexptype(), REALSXP | INTSXP | LGLSXP | CPLXSXP | RAWSXP) && !x.is_package_altrep()
}

/// `do_subset_dflt`: `[`'s default method, the builtin `.subset`. The
/// window holds `x`, the subscripts (`R_MissingArg` for an empty one) and
/// `drop =`.
// R: subset.c:do_subset_dflt
pub fn do_subset_dflt(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let n = args.len();
    // one index, not named: a scalar of a vector without attributes
    if n == 2 && args.name(cx, 1).is_none() {
        let x = args.get(cx, 0);
        if !attrib::has_attrib(x) && scalar_type(x) {
            let i = scalar_index(args.get(cx, 1));
            if i >= 1 && (i as usize) <= x.len().unwrap_or(0) {
                return cx.build(out, |cx| scalar_of(cx, args.get(cx, 0), i as usize - 1).unwrap());
            }
        }
    } else if n == 3 && args.name(cx, 1).is_none() && args.name(cx, 2).is_none() {
        // two indices, not named: an element of a matrix whose only
        // attribute is its dim
        let x = args.get(cx, 0);
        if let Some(a) = x.attrib().and_then(|a| a.as_cons())
            && a.tag().as_sym().is_some_and(|s| s.name().bytes() == b"dim")
            && a.cdr().sexptype() == NILSXP
            && let Some(d) = a.car().as_vector::<i32>()
            && d.len() == 2
            && scalar_type(x)
        {
            let (i, j) = (scalar_index(args.get(cx, 1)), scalar_index(args.get(cx, 2)));
            let (nrow, ncol) = (d.get(0) as i64, d.get(1) as i64);
            if i > 0 && j > 0 && i <= nrow && j <= ncol {
                let k = (i - 1 + nrow * (j - 1)) as usize;
                if k < x.len().unwrap_or(0) {
                    return cx.build(out, |cx| scalar_of(cx, args.get(cx, 0), k).unwrap());
                }
            }
        }
    }
    let sa = SubArgs::of(cx, args, true, false);
    let drop = drop_arg(cx, args, &sa);
    let Some((&xpos, subs)) = sa.pos.split_first() else {
        return Err(util::not_yet(cx, Some(call), "'[' without an object"));
    };
    let xr = args.root(xpos);
    let (t, is_vec, altrep) = {
        let x = xr.bind(cx);
        (x.sexptype(), attrib::is_vector(x), x.is_package_altrep())
    };
    if t == NILSXP {
        out.set(cx, cx.nil());
        return Ok(());
    }
    if altrep {
        return Err(util::not_yet(cx, Some(call), "'[' of an ALTREP vector"));
    }
    let nsubs = subs.len();
    cx.scope(|cx, s| {
        let ax = if is_vec {
            xr
        } else if matches!(t, LISTSXP | LANGSXP) {
            if attrib::dims(xr.bind(cx)).is_some_and(|d| d.len() > 1) {
                return Err(util::not_yet(cx, Some(call), "'[' of a pairlist with dimensions"));
            }
            let o = s.out(cx);
            let r = o.root();
            cx.build(o, |cx| pairs_to_list(cx, xr.bind(cx)))?;
            r
        } else {
            let msg = not_subsettable(xr.bind(cx));
            return Err(cx.error_call(Some(call), &msg));
        };
        let ndim = attrib::dims(xr.bind(cx)).map_or(0, |d| d.len());
        let ans = s.out(cx);
        let ansr = ans.root();
        if nsubs < 2 {
            let sub = if nsubs == 1 { args.root(subs[0]) } else { s.root(cx, cx.missing_arg()) };
            vector_subset(cx, ax, sub, call, ans)?;
            // one-dimensional arrays keep their dimension unless the result
            // has length one and drop is TRUE
            if ndim == 1 {
                let len = ansr.bind(cx).len().unwrap_or(0);
                if !drop || len > 1 {
                    let o = s.out(cx);
                    let r = o.root();
                    cx.build(o, |cx| one_d_array(cx, ansr.bind(cx), xr.bind(cx)))?;
                    ansr.replace_from(cx, r);
                }
            }
        } else {
            if nsubs != ndim {
                return Err(cx.error_call(Some(call), "incorrect number of dimensions"));
            }
            return Err(util::not_yet(cx, Some(call), "'[' with matrix or array subscripts"));
        }
        // pair-based objects are not coerced back, but calls are
        if t == LANGSXP {
            let o = s.out(cx);
            let r = o.root();
            cx.build(o, |cx| {
                let a = ansr.bind(cx);
                if a.len().unwrap_or(0) == 0 { Ok(cx.nil()) } else { list_to_call(cx, a) }
            })?;
            ansr.replace_from(cx, r);
        }
        // remove probably erroneous attributes
        for name in [&b"tsp"[..], b"class"] {
            if attrib::get(ansr.bind(cx), name).is_some() {
                let sym = install_sym(cx, s, name)?;
                cx.remove_attrib(ansr, sym)?;
            }
        }
        out.set(cx, ansr.bind(cx));
        Ok(())
    })
}

/// A 1-d array's subset keeps its dimension unless dropped: `dim` set to
/// the length (with the names of `x`'s dim), `dimnames` reinstated with the
/// subset's names.
fn one_d_array<'g>(cx: &'g Cx, ans: Ref<'g>, x: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
    let len = ans.len().unwrap_or(0);
    let mut a = Attrs::new();
    a.copy_all(ans);
    let nm = attrib::get_value(ans, b"names");
    let dim = attrib::get(x, b"dim").unwrap();
    let mut dv = cx.alloc_copy(&[len as i32])?;
    if let Some(dn) = attrib::get(dim.value, b"names") {
        let mut da = Attrs::new();
        da.install(dn);
        da.apply(cx, &mut dv)?;
    }
    a.set_dim(Attr { tag: dim.tag, value: dv.finish().into() });
    if let Some(dnames) = attrib::get(x, b"dimnames") {
        let old = dnames.value.as_vector::<Ref<'g>>().unwrap();
        let mut nd = cx.alloc_vector::<Ref<'g>>(old.len())?;
        for i in 0..old.len() {
            nd.set(i, old.get(i));
        }
        nd.set(0, nm.unwrap_or_else(|| cx.nil()));
        let mut da = Attrs::new();
        da.copy_all(dnames.value);
        da.apply(cx, &mut nd)?;
        a.install(Attr { tag: dnames.tag, value: nd.finish().into() });
        a.remove(b"names");
    }
    set_attrs(cx, strip(cx, ans)?, &a)
}

/// A copy of vector `x` without attributes.
fn strip<'g>(cx: &'g Cx, x: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
    let idx = Subscript::all(x.len().unwrap_or(0));
    extract_subset(cx, x, &idx)
}

/// What `do_subset2_dflt` found, for the caller to write.
enum Found<'g> {
    Value(Ref<'g>),
    Elt(Ref<'g>, usize),
    Null,
}

/// `do_subset2_dflt`: `[[`'s default method, the builtin `.subset2`. The
/// window holds `x`, the subscripts, and `exact =` / `drop =`.
// R: subset.c:do_subset2_dflt
pub fn do_subset2_dflt(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let sa = SubArgs::of(cx, args, true, true);
    let exact = exact_arg(cx, args, &sa);
    let pok = if exact == -1 { -1 } else { (exact == 0) as i32 };
    if sa.pos.is_empty() {
        return Err(cx.error_call(Some(call), "robigo: '[[' without an object is not implemented yet"));
    }
    let subs: Vec<usize> = sa.pos[1..].to_vec();
    let nsubs = subs.len();
    if nsubs == 0 {
        return Err(cx.error_call(Some(call), "no index specified"));
    }
    let x = args.get(cx, sa.pos[0]);
    if x.sexptype() == NILSXP {
        if args.get(cx, subs[0]) == cx.missing_arg() {
            return Err(cx.error_call(Some(call), subscript::MISSING_SUBSCRIPT));
        }
        out.set(cx, x);
        return Ok(());
    }
    let ndims = attrib::dims(x).map_or(0, |d| d.len());
    if nsubs > 1 && nsubs != ndims {
        return Err(cx.error_call(Some(call), "incorrect number of subscripts"));
    }
    match x.sexptype() {
        OBJSXP => return Err(util::not_yet(cx, Some(call), "'[[' of an S4 object")),
        ENVSXP => return subset_env(cx, call, args.root(sa.pos[0]), args.root(subs[0]), nsubs, out),
        _ => {}
    }
    if !(attrib::is_vector(x) || matches!(x.sexptype(), LISTSXP | LANGSXP)) {
        let msg = not_subsettable(x);
        return Err(cx.error_call(Some(call), &msg));
    }
    if x.is_package_altrep() {
        return Err(util::not_yet(cx, Some(call), "'[[' of an ALTREP vector"));
    }
    cx.scope(|cx, s| {
        let xr = s.root(cx, args.get(cx, sa.pos[0]));
        let offset = if nsubs == 1 {
            let thesub = args.root(subs[0]);
            let len = thesub.bind(cx).len().unwrap_or(0);
            if len > 1 {
                let getter = is_getter_call(cx, call);
                if getter && xr.bind(cx).maybe_shared() {
                    let o = s.out(cx);
                    let r = o.root();
                    cx.shallow_duplicate(xr, o)?;
                    xr.replace_from(cx, r);
                }
                vector_index(cx, xr, thesub, 0, len - 1, pok, call, getter)?;
            }
            let (off, warnings) = {
                let cx = &*cx;
                let xv = xr.bind(cx);
                let names = Names::of(cx, xv);
                get1index(cx, thesub.bind(cx), &names, xv.len().unwrap_or(0), pok, (len > 1).then(|| len - 1))
            }
            .map_err(|m| ecall(cx, Some(call), &m))?;
            for (m, with_call) in warnings {
                cx.warning_call(if with_call { Some(call) } else { None }, &m)?;
            }
            let xv = xr.bind(cx);
            let nx = xv.len().unwrap_or(0) as i64;
            if off < 0 || off >= nx {
                if off < 0 && matches!(xv.sexptype(), VECSXP | EXPRSXP | NILSXP | LISTSXP | LANGSXP) {
                    out.set(cx, cx.nil());
                    return Ok(());
                }
                return Err(cx.error_call(Some(call), OUT_OF_BOUNDS));
            }
            off as usize
        } else {
            // matrix or array indexing: one position per dimension
            let mut off = 0i64;
            let mut pos = Vec::with_capacity(nsubs);
            for (i, &si) in subs.iter().enumerate() {
                let r = {
                    let cx = &*cx;
                    let xv = xr.bind(cx);
                    let dims = attrib::dims(xv).unwrap();
                    let dn = attrib::get_value(xv, b"dimnames");
                    let names = match dn.and_then(|d| d.as_vector::<Ref<'_>>()) {
                        Some(l) if i < l.len() => Names::of_vector(l.get(i)),
                        _ => Names::None,
                    };
                    get1index(cx, args.get(cx, si), &names, dims[i] as usize, pok, None).map(|(o, _)| (o, dims[i]))
                };
                let (p, d) = r.map_err(|m| ecall(cx, Some(call), &m))?;
                if p < 0 || p >= d as i64 {
                    return Err(cx.error_call(Some(call), OUT_OF_BOUNDS));
                }
                pos.push(p);
            }
            let dims = attrib::dims(xr.bind(cx)).unwrap().to_vec();
            for i in (1..nsubs).rev() {
                off = (off + pos[i]) * dims[i - 1] as i64;
            }
            (off + pos[0]) as usize
        };
        cx.build(out, |cx| {
            let xv = xr.bind(cx);
            Ok(match found(cx, xv, offset) {
                Found::Value(v) => v,
                Found::Elt(v, i) => scalar_of(cx, v, i).unwrap_or_else(|| element_string(cx, v, i))?,
                Found::Null => cx.nil(),
            })
        })
    })
}

/// Element `i` of a character vector as a new one.
fn element_string<'g>(cx: &'g Cx, v: Ref<'g>, i: usize) -> Result<Ref<'g>, NeedGc> {
    let c = v.as_vector::<Char<'g>>().unwrap().get(i);
    let mut r = cx.alloc_vector::<Char<'g>>(1)?;
    r.set(0, c);
    Ok(r.finish().into())
}

/// `[[`'s element at `offset` of `x`.
fn found<'g>(cx: &'g Cx, x: Ref<'g>, offset: usize) -> Found<'g> {
    let _ = cx;
    match x.view() {
        View::Pairlist(_) | View::Lang(_) | View::Dots(_) => {
            let mut c = x.as_cons();
            for _ in 0..offset {
                c = c.and_then(|cell| cell.cdr().as_cons());
            }
            c.map_or(Found::Null, |cell| Found::Value(cell.car()))
        }
        View::List(v) | View::Expr(v) => Found::Value(v.get(offset)),
        _ => Found::Elt(x, offset),
    }
}

/// `IS_GETTER_CALL(call)`: the call's first argument is `*tmp*`.
fn is_getter_call(cx: &Cx, call: Root<'_>) -> bool {
    call.bind(cx)
        .as_cons()
        .and_then(|c| c.cdr().as_cons())
        .and_then(|c| c.car().as_sym())
        .is_some_and(|s| s.name().bytes() == b"*tmp*")
}

/// `vectorIndex(x, thesub, start, stop, pok, call, dup)`: index the list
/// or pairlist in root `x` recursively by `thesub`'s elements `start..stop`;
/// the root ends as the element reached. With `dup` (an assignment), an
/// element that may be shared is duplicated and stored back first.
// R: subscript.c:vectorIndex
#[allow(clippy::too_many_arguments)]
pub fn vector_index(
    cx: &mut Cx,
    x: Root<'_>,
    thesub: Root<'_>,
    start: usize,
    stop: usize,
    pok: i32,
    call: Root<'_>,
    dup: bool,
) -> RResult<()> {
    if dup && x.bind(cx).maybe_shared() {
        return Err(cx.error("should only be called in an assignment context."));
    }
    for i in start..stop {
        let xv = x.bind(cx);
        let t = xv.sexptype();
        if !matches!(t, VECSXP | EXPRSXP | NILSXP | LISTSXP) {
            let msg = if i > 0 {
                format!("recursive indexing failed at level {}\n", i + 1)
            } else {
                "attempt to select more than one element in vectorIndex".to_owned()
            };
            return Err(cx.error_call(Some(call), &msg));
        }
        let r = {
            let cx = &*cx;
            let xv = x.bind(cx);
            let names = Names::of(cx, xv);
            get1index(cx, thesub.bind(cx), &names, xv.len().unwrap_or(0), pok, Some(i))
        };
        let (offset, _) = r.map_err(|m| ecall(cx, Some(call), &m))?;
        let nx = x.bind(cx).len().unwrap_or(0) as i64;
        if offset < 0 || offset >= nx {
            let msg = format!("no such index at level {}\n", i + 1);
            return Err(cx.error_call(Some(call), &msg));
        }
        let offset = offset as usize;
        cx.scope(|cx, s| {
            let container = s.root(cx, x.bind(cx));
            let e = match found(cx, container.bind(cx), offset) {
                Found::Value(e) => e,
                _ => unreachable!(),
            };
            let er = s.root(cx, e);
            if dup && e.maybe_shared() {
                let o = s.out(cx);
                let d = o.root();
                cx.shallow_duplicate(er, o)?;
                if container.bind(cx).sexptype() == LISTSXP {
                    let mut c = container.bind(cx).as_cons().unwrap();
                    for _ in 0..offset {
                        c = c.cdr().as_cons().unwrap();
                    }
                    let cell = s.root(cx, c.as_ref());
                    cx.set_car(cell, d);
                } else {
                    cx.unique_list(container)?.set(offset, d);
                }
                er.replace_from(cx, d);
            }
            x.replace_from(cx, er);
            Ok(())
        })?;
    }
    Ok(())
}

/// `[[` of an environment: one string names the binding; a promise is
/// forced, an unbound name is NULL.
fn subset_env(cx: &mut Cx, call: Root<'_>, env: Root<'_>, sub: Root<'_>, nsubs: usize, out: Out<'_>) -> RResult<()> {
    let name = match sub.bind(cx).as_vector::<Char<'_>>() {
        Some(v) if nsubs == 1 && v.len() == 1 => crate::text::to_native(crate::coerce::text(v.get(0)), cx.locale())
            .map(|b| b.into_owned())
            .unwrap_or_else(|_| v.get(0).bytes().to_vec()),
        _ => return Err(cx.error_call(Some(call), "wrong arguments for subsetting an environment")),
    };
    env_value(cx, env, &name, out)
}

/// `R_findVarInFrame(env, install(name))`, a promise forced (GNU R's
/// `eval(ans, R_GlobalEnv)`), unbound as NULL.
pub fn env_value(cx: &mut Cx, env: Root<'_>, name: &[u8], out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let sym = install_sym(cx, s, name)?;
        let v = s.out(cx);
        let vr = v.root();
        cx.find_var_in_frame3(env, sym, true, v)?;
        let val = vr.bind(cx);
        if val == cx.unbound() {
            out.set(cx, cx.nil());
        } else if val.sexptype() == PROMSXP {
            cx.force(vr, out)?;
        } else {
            out.set(cx, val);
        }
        Ok(())
    })
}

/// Element `i` of the list in root `x`.
fn list_elt<'g>(cx: &'g Cx, x: Root<'_>, i: usize) -> Ref<'g> {
    x.bind(cx).as_vector::<Ref<'g>>().unwrap().get(i)
}

/// `pstrmatch(target, input, slen)`: an exact match, a partial one, or none
/// of the name `target` (a symbol or CHARSXP) against `input`.
// R: subset.c:pstrmatch
fn pstrmatch(target: Option<&[u8]>, input: &[u8]) -> u8 {
    let Some(st) = target else { return 0 };
    if !input.is_empty() && st.starts_with(input) { if st.len() == input.len() { 2 } else { 1 } } else { 0 }
}

/// `R_subset3_dflt(x, input, call)`: `$`'s default method; `input` is the
/// name (a CHARSXP in root `input`).
// R: subset.c:R_subset3_dflt
pub fn subset3_dflt(cx: &mut Cx, x: Root<'_>, input: Root<'_>, call: Root<'_>, out: Out<'_>) -> RResult<()> {
    let (xt, s4, alt) = {
        let xv = x.bind(cx);
        (xv.sexptype(), xv.is_s4(), xv.is_package_altrep())
    };
    let inp = {
        let c = input.bind(cx).as_char().expect("robigo: '$' takes a CHARSXP name");
        crate::text::to_native(crate::coerce::text(c), cx.locale())
            .map(|b| b.into_owned())
            .unwrap_or(c.bytes().to_vec())
    };
    match xt {
        OBJSXP if s4 => Err(util::not_yet(cx, Some(call), "'$' of an S4 object")),
        LISTSXP | LANGSXP | NILSXP => {
            let getter = is_getter_call(cx, call);
            cx.scope(|cx, s| {
                // the exact match's value, or the partially matched cells
                let mut cells = Vec::new();
                let mut exact = None;
                {
                    let cx = &*cx;
                    let mut c = x.bind(cx).as_cons();
                    while let Some(cell) = c {
                        let tag = cell.tag().as_sym().map(|s| s.name().bytes());
                        match pstrmatch(tag, &inp) {
                            2 => {
                                exact = Some(s.root(cx, cell.car()));
                                break;
                            }
                            1 => cells.push(s.root(cx, cell.as_ref())),
                            _ => {}
                        }
                        c = cell.cdr().as_cons();
                    }
                }
                if let Some(v) = exact {
                    out.set(cx, v.bind(cx));
                    return;
                }
                // R_warn_partial_match_dollar is options(warnPartialMatchDollar),
                // FALSE by default; options are B2b's
                if getter {
                    for &c in &cells {
                        cx.mark_not_mutable(c);
                    }
                }
                let v = if cells.len() == 1 { cells[0].bind(cx).as_cons().unwrap().car() } else { cx.nil() };
                out.set(cx, v);
            });
            Ok(())
        }
        VECSXP | EXPRSXP => {
            if alt {
                return Err(util::not_yet(cx, Some(call), "'$' of an ALTREP list"));
            }
            let getter = is_getter_call(cx, call);
            let (exact, first, nmatch) = {
                let cx = &*cx;
                let names = attrib::names(x.bind(cx)).and_then(|n| n.as_vector::<Char<'_>>());
                let n = names.map_or(0, |v| v.len());
                let (mut exact, mut first, mut nmatch) = (None, None, 0);
                for i in 0..n {
                    let nm = names.unwrap().get(i);
                    let b = crate::text::to_native(crate::coerce::text(nm), cx.locale())
                        .map(|b| b.into_owned())
                        .unwrap_or(nm.bytes().to_vec());
                    match pstrmatch(Some(&b), &inp) {
                        2 => {
                            exact = Some(i);
                            break;
                        }
                        1 => {
                            nmatch += 1;
                            first = first.or(Some(i));
                        }
                        _ => {}
                    }
                }
                (exact, first, nmatch)
            };
            if let Some(i) = exact {
                let v = list_elt(cx, x, i);
                out.set(cx, v);
                return Ok(());
            }
            // the first partial match is marked not mutable in a getter call
            // ("partial matches can cause aliasing in eval.c:evalseq")
            if getter && let Some(i) = first {
                cx.scope(|cx, s| {
                    let v = s.root(cx, list_elt(cx, x, i));
                    cx.mark_not_mutable(v);
                });
            }
            let v = if nmatch == 1 { list_elt(cx, x, first.unwrap()) } else { cx.nil() };
            out.set(cx, v);
            Ok(())
        }
        ENVSXP => env_value(cx, x, &inp, out),
        LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP => {
            Err(cx.error_call(Some(call), "$ operator is invalid for atomic vectors"))
        }
        _ => {
            let msg = not_subsettable(x.bind(cx));
            Err(cx.error_call(Some(call), &msg))
        }
    }
}
