//! attrib.c's writers (B2a): GNU R's `setAttrib` with the setters of its
//! special names (`namesgets`, `dimgets`, `dimnamesgets`, `classgets`,
//! `tspgets`, `commentgets`, `row_names_gets`), and the replacement
//! builtins over them: `attr<-`, `attributes<-`, `oldClass<-`, `class<-`
//! (coerce.c's `R_set_class`), `unclass` (objects.c), `names<-`, `dim<-`,
//! `dimnames<-`, `levels<-` and `.Internal(comment<-)`.
//!
//! A setter writes the object in its root in place, as GNU R's do: the
//! caller has made it one it may write (a builtin by GNU R's rule,
//! [`writable`]). Values the setters store are rooted throughout; they
//! reach the object through the facade's writers (`install_attrib`,
//! `remove_attrib`, `set_tag`, ...), which keep the barrier and the counts.

use super::{dims, get_value, has_attrib, inherits, is_array, is_vector};
use crate::coerce::vector::{NA_INTEGER, as_character_factor, as_real, coerce, coerce_vector};
use crate::coerce::{dispatch_or_eval, xlength};
use crate::registry;
use crate::util::{is_numeric, is_vector_atomic, not_yet, type_to_char};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Built, Char, Cx, Diag, Dispatched, Generic, Out, RResult, Ref, Root, Scope, View};

// ---- names ------------------------------------------------------------------

/// `install(name)` with its errors, rooted in `s` (setAttrib's
/// `installTrChar` of a name the caller translated).
// R: names.c:install
fn install_name<'s>(cx: &mut Cx, s: Scope<'s>, name: &[u8]) -> RResult<Root<'s>> {
    if name.is_empty() {
        return Err(cx.error("attempt to use zero-length variable name"));
    }
    if name.len() > 10000 {
        return Err(cx.error("variable names are limited to 10000 bytes"));
    }
    cx.install_rooted(s, name)
}

/// `translateChar(c)`: an attribute name in the native encoding.
fn translated(cx: &mut Cx, c: Root<'_>, i: usize) -> RResult<Vec<u8>> {
    let r = {
        let ch = c.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
        crate::text::to_native(crate::coerce::text(ch), cx.locale()).map(|b| b.into_owned())
    };
    r.map_err(|e| cx.error(&e.to_string()))
}

// ---- setAttrib --------------------------------------------------------------

/// `setAttrib(vec, name, val)`, `name` a symbol's name: NULL removes the
/// attribute; the special names go to their setters; any other is
/// installed.
// R: attrib.c:setAttrib
pub fn set_attrib(cx: &mut Cx, x: Root<'_>, name: &[u8], val: Root<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let sym = install_name(cx, s, name)?;
        if val.bind(cx).sexptype() == NILSXP {
            return cx.remove_attrib(x, sym);
        }
        if x.bind(cx).sexptype() == NILSXP {
            return Err(cx.error("attempt to set an attribute on NULL"));
        }
        match name {
            b"names" => namesgets(cx, x, val),
            b"dim" => dimgets(cx, x, val),
            b"dimnames" => dimnamesgets(cx, x, val),
            b"class" => classgets(cx, x, val),
            b"tsp" => tspgets(cx, x, val),
            b"comment" => commentgets(cx, x, val),
            b"row.names" => row_names_gets(cx, x, val),
            _ => install_attrib(cx, x, sym, val),
        }
    })
}

/// `installAttrib(vec, name, val)`: a value replacing another goes through
/// `R_FixupRHS` (copied if it contains the object).
// R: attrib.c:installAttrib
fn install_attrib(cx: &mut Cx, x: Root<'_>, name: Root<'_>, val: Root<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let v = s.root(cx, val.bind(cx));
        let fixup = {
            let (xv, n, vv) = (x.bind(cx), name.bind(cx), v.bind(cx));
            super::attrs(xv).find(|a| a.tag.as_ref() == n).is_some_and(|a| vv.maybe_referenced() && a.value != vv)
        };
        if fixup {
            crate::subassign::fixup_rhs(cx, x, v)?;
        }
        cx.install_attrib(x, name, v)
    })
}

/// `install(name)` then [`install_attrib`].
fn install_named(cx: &mut Cx, x: Root<'_>, name: &[u8], val: Root<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let n = cx.install_rooted(s, name)?;
        install_attrib(cx, x, n, val)
    })
}

/// `removeAttrib(vec, name)` for a name robigo spells.
fn remove_named(cx: &mut Cx, x: Root<'_>, name: &[u8]) -> RResult<()> {
    cx.scope(|cx, s| {
        let n = cx.install_rooted(s, name)?;
        cx.remove_attrib(x, n)
    })
}

/// GNU R's `isOneDimensionalArray`: a vector, pairlist or call whose `dim`
/// is an integer vector of length one.
// R: attrib.c:isOneDimensionalArray
fn one_dimensional(x: Ref<'_>) -> bool {
    (is_vector(x) || matches!(x.sexptype(), NILSXP | LISTSXP | LANGSXP))
        && get_value(x, b"dim").is_some_and(|d| d.sexptype() == INTSXP && d.len() == Some(1))
}

/// `namesgets(vec, val)`: `val` as a character vector (a pairlist's
/// elements one by one), recycled with NA to the object's length, checked
/// (`checkNames`), then set: a one-dimensional array's dimnames, a
/// pairlist's or call's tags, else the names attribute.
// R: attrib.c:namesgets, attrib.c:checkNames
pub fn namesgets(cx: &mut Cx, x: Root<'_>, val: Root<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let o = s.out(cx);
        let v = o.root();
        if val.bind(cx).sexptype() == LISTSXP {
            let ok = {
                let mut c = val.bind(cx).as_cons();
                let mut ok = true;
                while let Some(cell) = c {
                    let e = cell.car();
                    ok &= is_vector(e) && xlength(e) <= 1;
                    c = cell.cdr().as_cons();
                }
                ok
            };
            if !ok {
                return Err(cx.error("incompatible 'names' argument"));
            }
            let n = xlength(x.bind(cx));
            cx.build_with(o, |cx| {
                let mut r = cx.alloc_vector::<Char<'_>>(n)?;
                let mut c = val.bind(cx).as_cons();
                let mut i = 0;
                while i < n
                    && let Some(cell) = c
                {
                    let sv = match coerce(cx, cell.car(), STRSXP, true)? {
                        Built::Done(sv, _) => sv,
                        Built::Fail(d) => return Ok(Built::fail(Diag::error(d.message().to_owned()))),
                    };
                    match sv.as_vector::<Char<'_>>() {
                        Some(sv) if !sv.is_empty() => r.set(i, sv.get(0)),
                        // GNU R reads STRING_ELT(s, 0) of an empty vector
                        _ => {
                            let m = "robigo: 'names<-' from a pairlist with an empty element is not implemented yet";
                            return Ok(Built::fail(Diag::error(m.to_owned())));
                        }
                    }
                    i += 1;
                    c = cell.cdr().as_cons();
                }
                Ok(Built::done(r.finish()))
            })?;
        } else {
            coerce_vector(cx, val, STRSXP, o)?;
        }
        let nx = xlength(x.bind(cx));
        if xlength(v.bind(cx)) < nx {
            let o = s.out(cx);
            let r = o.root();
            cx.build(o, |cx| crate::seq::xlengthgets(cx, v.bind(cx), nx))?;
            v.replace_from(cx, r);
        }
        // checkNames
        let (xt, s4) = (x.bind(cx).sexptype(), x.bind(cx).is_s4());
        if is_vector(x.bind(cx)) || matches!(xt, LISTSXP | LANGSXP) {
            let nv = xlength(v.bind(cx));
            if nx != nv {
                let msg = format!("'names' attribute [{nv}] must be the same length as the vector [{nx}]");
                return Err(cx.error(&msg));
            }
        } else if !s4 {
            return Err(cx.error("names() applied to a non-vector"));
        }
        if one_dimensional(x.bind(cx)) {
            let o = s.out(cx);
            let l = o.root();
            cx.build(o, |cx| cx.cons(v.bind(cx), cx.nil()))?;
            return dimnamesgets(cx, x, l);
        }
        if matches!(xt, LISTSXP | LANGSXP) {
            let cur = s.root(cx, x.bind(cx));
            let mut i = 0;
            while cur.bind(cx).as_cons().is_some() {
                let tag = {
                    let cx = &*cx;
                    let c = v.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
                    c != cx.na_string() && !c.bytes().is_empty()
                };
                let t = if tag {
                    let b = translated(cx, v, i)?;
                    cx.install_rooted(s, &b)?
                } else {
                    s.root(cx, cx.nil())
                };
                cx.set_tag(cur, t);
                let next = s.root(cx, cur.bind(cx).as_cons().unwrap().cdr());
                cur.replace_from(cx, next);
                i += 1;
            }
            return Ok(());
        }
        install_named(cx, x, b"names", v)
    })
}

/// `dimgets(vec, val)`: `val` as integers (`coerceVector`), whose product
/// must be the length; the dimnames go, the dim is installed and marked not
/// mutable.
// R: attrib.c:dimgets, array.c:dim2total
pub fn dimgets(cx: &mut Cx, x: Root<'_>, val: Root<'_>) -> RResult<()> {
    let xv = x.bind(cx);
    if !is_vector(xv) && !matches!(xv.sexptype(), NILSXP | LISTSXP) {
        return Err(cx.error("invalid first argument, must be vector (list or atomic)"));
    }
    let vv = val.bind(cx);
    if vv.sexptype() != NILSXP && !is_vector_atomic(vv) {
        return Err(cx.error("invalid second argument, must be vector or NULL"));
    }
    cx.scope(|cx, s| {
        let o = s.out(cx);
        let d = o.root();
        coerce_vector(cx, val, INTSXP, o)?;
        let total = {
            let Some(dv) = d.bind(cx).as_ints() else {
                return Err(not_yet(cx, None, "'dim<-' with a package ALTREP value"));
            };
            if dv.is_empty() {
                return Err(cx.error("'dim' cannot be of length 0"));
            }
            let mut dn = 1f64;
            for &e in dv {
                if e == NA_INTEGER {
                    return Err(cx.error("the dims contain missing values"));
                }
                if e < 0 {
                    return Err(cx.error("the dims contain negative values"));
                }
                dn = if e != 0 { dn * e as f64 } else { 0.0 };
            }
            // R_XLEN_T_MAX
            if dn > 4503599627370496.0 {
                return Err(cx.error("too many elements specified"));
            }
            dn as u64
        };
        let len = xlength(x.bind(cx)) as u64;
        if total != len {
            let msg = format!("dims [product {total}] do not match the length of object [{len}]");
            return Err(cx.error(&msg));
        }
        remove_named(cx, x, b"dimnames")?;
        install_named(cx, x, b"dim", d)?;
        cx.mark_not_mutable(d);
        Ok(())
    })
}

/// `as_char_simpl(val1)`: a dimnames element as a character vector: NULL
/// when empty, a factor's labels, a string itself, else `coerceVector`
/// without attributes.
// R: attrib.c:as_char_simpl
fn as_char_simpl<'s>(cx: &mut Cx, s: Scope<'s>, v: Root<'_>) -> RResult<Root<'s>> {
    let vv = v.bind(cx);
    if xlength(vv) == 0 {
        return Ok(s.root(cx, cx.nil()));
    }
    if inherits(vv, b"factor") {
        let o = s.out(cx);
        let r = o.root();
        cx.build_with(o, |cx| {
            Ok(match as_character_factor(cx, v.bind(cx))? {
                Ok(f) => Built::done(f),
                Err(m) => Built::fail(Diag::error(m)),
            })
        })?;
        return Ok(r);
    }
    if vv.sexptype() == STRSXP {
        return Ok(s.root(cx, vv));
    }
    let o = s.out(cx);
    let r = o.root();
    let w = cx.build_with(o, |cx| coerce(cx, v.bind(cx), STRSXP, false))?;
    w.signal(cx)?;
    // SET_ATTRIB(this2, R_NilValue); SET_OBJECT(this2, 0): a fresh result
    if has_attrib(r.bind(cx)) || r.bind(cx).is_object() {
        let nil = s.root(cx, cx.nil());
        cx.set_attrib_list(r, nil);
        cx.set_object_bit(r, false);
    }
    Ok(r)
}

/// `dimnamesgets(vec, val)`: a list (a pairlist made one) of one element
/// per extent, padded with NULL, each NULL or a vector of the extent's
/// length made character; installed and marked not mutable. A pairlist
/// with one extent takes the names as its tags.
// R: attrib.c:dimnamesgets
pub fn dimnamesgets(cx: &mut Cx, x: Root<'_>, val: Root<'_>) -> RResult<()> {
    let xv = x.bind(cx);
    let x_pairs = matches!(xv.sexptype(), NILSXP | LISTSXP);
    if !is_array(xv) && !x_pairs {
        return Err(cx.error("'dimnames' applied to non-array"));
    }
    let vt = val.bind(cx).sexptype();
    if !matches!(vt, NILSXP | LISTSXP | VECSXP) {
        return Err(cx.error("'dimnames' must be a list"));
    }
    let extents: Vec<i32> = dims(xv).map(<[i32]>::to_vec).unwrap_or_default();
    let k = extents.len();
    let lv = xlength(val.bind(cx));
    if k < lv {
        return Err(cx.error(&format!("length of 'dimnames' [{lv}] must match that of 'dims' [{k}]")));
    }
    if lv == 0 {
        return remove_named(cx, x, b"dimnames");
    }
    cx.scope(|cx, s| {
        let v = s.root(cx, val.bind(cx));
        if vt == LISTSXP {
            // old list to new list
            let o = s.out(cx);
            let r = o.root();
            cx.build(o, |cx| {
                let mut l = cx.alloc_vector::<Ref<'_>>(k)?;
                let mut c = v.bind(cx);
                for i in 0..k {
                    let cell = c.as_cons();
                    l.set(i, cell.map_or(cx.nil(), |c| c.car()));
                    c = cell.map_or(cx.nil(), |c| c.cdr());
                }
                Ok(l.finish().into())
            })?;
            v.replace_from(cx, r);
        }
        let n = xlength(v.bind(cx));
        if n > 0 && n < k {
            let o = s.out(cx);
            let r = o.root();
            cx.build(o, |cx| crate::seq::xlengthgets(cx, v.bind(cx), k))?;
            v.replace_from(cx, r);
        }
        if v.bind(cx).maybe_referenced() {
            let o = s.out(cx);
            let r = o.root();
            cx.shallow_duplicate(v, o)?;
            v.replace_from(cx, r);
        }
        let n = xlength(v.bind(cx));
        if k != n {
            return Err(cx.error(&format!("length of 'dimnames' [{n}] must match that of 'dims' [{k}]")));
        }
        for (i, &extent) in extents.iter().enumerate() {
            let this = {
                let View::List(l) = v.bind(cx).view() else { unreachable!("robigo: made a list above") };
                s.root(cx, l.get(i))
            };
            let t = this.bind(cx);
            if t.sexptype() == NILSXP {
                continue;
            }
            if !is_vector(t) {
                let msg = format!("invalid type ({}) for 'dimnames' (must be a vector)", type_to_char(t));
                return Err(cx.error(&msg));
            }
            let len = xlength(t);
            if extent as usize != len && len != 0 {
                return Err(cx.error(&format!("length of 'dimnames' [{}] not equal to array extent", i + 1)));
            }
            let c = as_char_simpl(cx, s, this)?;
            cx.unique_list(v)?.set(i, c);
        }
        install_named(cx, x, b"dimnames", v)?;
        if x_pairs && k == 1 {
            let top = {
                let View::List(l) = v.bind(cx).view() else { unreachable!("robigo: made a list above") };
                s.root(cx, l.get(0))
            };
            if top.bind(cx).sexptype() != STRSXP {
                let msg = format!(
                    "STRING_ELT() can only be applied to a 'character vector', not a '{}'",
                    type_to_char(top.bind(cx))
                );
                return Err(cx.error(&msg));
            }
            let cur = s.root(cx, x.bind(cx));
            let mut i = 0;
            while cur.bind(cx).as_cons().is_some() {
                let b = translated(cx, top, i)?;
                let t = install_name(cx, s, &b)?;
                cx.set_tag(cur, t);
                let next = s.root(cx, cur.bind(cx).as_cons().unwrap().cdr());
                cur.replace_from(cx, next);
                i += 1;
            }
        }
        cx.mark_not_mutable(v);
        Ok(())
    })
}

/// `classgets(vec, klass)`: a character vector or NULL; an empty one
/// removes the class and clears OBJECT, any other is installed and sets
/// it (`"factor"` only on an integer vector).
// R: attrib.c:classgets
pub fn classgets(cx: &mut Cx, x: Root<'_>, klass: Root<'_>) -> RResult<()> {
    let k = klass.bind(cx);
    if !matches!(k.sexptype(), NILSXP | STRSXP) {
        return Err(cx.error("attempt to set invalid 'class' attribute"));
    }
    if xlength(k) == 0 {
        // stripAttrib, SET_OBJECT(vec, 0)
        return remove_named(cx, x, b"class");
    }
    let factor = {
        let kv = k.as_vector::<Char<'_>>().unwrap();
        (0..kv.len()).any(|i| kv.get(i).bytes() == b"factor")
    };
    if factor && x.bind(cx).sexptype() != INTSXP {
        return Err(cx.error("adding class \"factor\" to an invalid object"));
    }
    install_named(cx, x, b"class", klass)?;
    cx.set_object_bit(x, true);
    Ok(())
}

/// GNU R's `nrows(s)`: a vector's or pairlist's first extent, else its
/// length.
// R: util.c:nrows
fn nrows(cx: &mut Cx, x: Root<'_>) -> RResult<usize> {
    let x = x.bind(cx);
    if is_vector(x) || matches!(x.sexptype(), NILSXP | LISTSXP) {
        return Ok(match dims(x) {
            Some(d) => d.first().copied().unwrap_or(0).max(0) as usize,
            None => xlength(x),
        });
    }
    Err(cx.error("object is not a matrix"))
}

/// `tspgets(vec, val)`: three numbers (start, end, frequency) consistent
/// with the number of rows to within `getOption("ts.eps")`, installed as
/// doubles.
// R: attrib.c:tspgets, attrib.c:badtsp
fn tspgets(cx: &mut Cx, x: Root<'_>, val: Root<'_>) -> RResult<()> {
    let vv = val.bind(cx);
    if x.bind(cx).is_s4() {
        if !is_numeric(vv) {
            return Err(cx.error("'tsp' attribute must be numeric"));
        }
        return install_named(cx, x, b"tsp", val);
    }
    if !is_numeric(vv) || vv.len() != Some(3) {
        return Err(cx.error("'tsp' attribute must be numeric of length three"));
    }
    let p: [f64; 3] = match vv.view() {
        View::Real(r) => [r.get(0), r.get(1), r.get(2)],
        _ => {
            // INTEGER(val) of an integer or logical vector
            let i = vv.as_ints().unwrap();
            let f = |e: i32| if e == NA_INTEGER { robigo_text::NA_REAL } else { e as f64 };
            [f(i[0]), f(i[1]), f(i[2])]
        }
    };
    let [start, end, frequency] = p;
    let bad = |cx: &mut Cx, k: i32| cx.error(&format!("invalid time series parameters specified ({k})"));
    if frequency <= 0.0 {
        return Err(bad(cx, 0));
    }
    let n = nrows(cx, x)?;
    if n == 0 {
        return Err(cx.error("cannot assign 'tsp' to zero-length vector"));
    }
    cx.scope(|cx, s| {
        let eps = {
            let o = crate::options::get_option(cx, s, b"ts.eps")?;
            as_real(cx, o.bind(cx)).0
        };
        // (NA when the option is unset, as with no stats: never bad)
        if (end - start - (n - 1) as f64 / frequency).abs() > eps {
            return Err(bad(cx, 1));
        }
        let o = s.out(cx);
        let t = o.root();
        cx.build(o, |cx| Ok(cx.alloc_copy(&[start, end, frequency])?.finish().into()))?;
        install_named(cx, x, b"tsp", t)
    })
}

/// `commentgets(vec, comment)`: a character vector or NULL; an empty one
/// removes the comment.
// R: attrib.c:commentgets
fn commentgets(cx: &mut Cx, x: Root<'_>, comment: Root<'_>) -> RResult<()> {
    let c = comment.bind(cx);
    if !matches!(c.sexptype(), NILSXP | STRSXP) {
        return Err(cx.error("attempt to set invalid 'comment' attribute"));
    }
    if xlength(c) == 0 { remove_named(cx, x, b"comment") } else { install_named(cx, x, b"comment", comment) }
}

/// `row_names_gets(vec, val)`: character or integer row names; `1:n` (or
/// `c(NA, n)`, or a double `c(NA, n)` a careless `dput` leaves) is stored
/// in the compact form `c(NA_integer_, n)`.
// R: attrib.c:row_names_gets
fn row_names_gets(cx: &mut Cx, x: Root<'_>, val: Root<'_>) -> RResult<()> {
    let vv = val.bind(cx);
    if let View::Real(r) = vv.view()
        && r.len() == 2
        && r.get(0).is_nan()
    {
        return cx.scope(|cx, s| {
            let o = s.out(cx);
            let v = o.root();
            coerce_vector(cx, val, INTSXP, o)?;
            install_named(cx, x, b"row.names", v)
        });
    }
    // isInteger: not a factor
    if vv.sexptype() == INTSXP && !inherits(vv, b"factor") {
        let Some(iv) = vv.as_ints() else {
            return Err(not_yet(cx, None, "row names from a package ALTREP vector"));
        };
        let n = iv.len();
        let compact = if n == 2 && iv[0] == NA_INTEGER {
            Some(iv[1])
        } else if n > 2 {
            iv.iter().enumerate().all(|(i, &e)| e as i64 == i as i64 + 1).then_some(n as i32)
        } else {
            None
        };
        if let Some(n) = compact {
            // the length hidden in an impossible integer vector
            return cx.scope(|cx, s| {
                let o = s.out(cx);
                let v = o.root();
                cx.build(o, |cx| Ok(cx.alloc_copy(&[NA_INTEGER, n])?.finish().into()))?;
                install_named(cx, x, b"row.names", v)
            });
        }
    } else if vv.sexptype() != STRSXP {
        let msg = format!("row names must be 'character' or 'integer', not '{}'", type_to_char(vv));
        return Err(cx.error(&msg));
    }
    install_named(cx, x, b"row.names", val)
}

// ---- the replacement builtins -------------------------------------------------

/// How a replacement builtin copies its object before writing it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Copy {
    /// `if (MAYBE_SHARED(x) || (!IS_ASSIGNMENT_CALL(call) &&
    /// MAYBE_REFERENCED(x))) x = shallow_duplicate(x)`: the setters'
    /// rule (`R_shallow_duplicate_attr` is a shallow duplicate here).
    Setter,
    /// The same rule with a deep `duplicate` (`levels<-`).
    SetterDeep,
    /// `if (MAYBE_SHARED(x)) x = duplicate(x)` (`comment<-`).
    SharedDeep,
    /// `if (MAYBE_REFERENCED(x)) x = shallow_duplicate(x)` (`unclass`).
    Referenced,
}

/// The builtin's object `x`, or the copy GNU R's rule `how` makes, in a
/// root of `s` the builtin may write.
fn writable<'s>(cx: &mut Cx, s: Scope<'s>, call: Root<'_>, x: Root<'_>, how: Copy) -> RResult<Root<'s>> {
    let r = s.root(cx, x.bind(cx));
    let (shared, referenced) = (r.bind(cx).maybe_shared(), r.bind(cx).maybe_referenced());
    let outside = !call.bind(cx).is_assignment_call();
    let copy = match how {
        Copy::Setter | Copy::SetterDeep => shared || (outside && referenced),
        Copy::SharedDeep => shared,
        Copy::Referenced => referenced,
    };
    if copy {
        let o = s.out(cx);
        let d = o.root();
        if matches!(how, Copy::SetterDeep | Copy::SharedDeep) {
            cx.duplicate(r, o)?;
        } else {
            cx.shallow_duplicate(r, o)?;
        }
        r.replace_from(cx, d);
    }
    Ok(r)
}

/// `attr(x, which) <- value`: `x` is the first argument as given (GNU R
/// takes `CAR(args)` before matching), `which` and `value` are matched to
/// `(x, which, value)`.
// R: attrib.c:do_attrgets (PRIMVAL 0)
pub fn do_attrgets(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::by_name("attr<-").unwrap().check_arity(cx, call, args)?;
    cx.scope(|cx, s| {
        let obj = writable(cx, s, call, args.root(0), Copy::Setter)?;
        let m = crate::util::match_args(cx, call, &["x", "which", "value"], args)?;
        let (Some(wi), Some(vi)) = (m.formals[1], m.formals[2]) else {
            unreachable!("robigo: three arguments fill three formals")
        };
        let name = args.root(wi);
        let valid = {
            let n = name.bind(cx);
            n.as_vector::<Char<'_>>().filter(|v| !v.is_empty()).map(|v| (v.get(0) != cx.na_string(), v.len()))
        };
        match valid {
            Some((true, 1)) => {}
            Some((true, _)) => return Err(cx.error("'name' must be a scalar string")),
            _ => return Err(cx.error("'name' must be non-null character string")),
        }
        let nm = translated(cx, name, 0)?;
        set_attrib(cx, obj, &nm, args.root(vi))?;
        out.set(cx, obj.bind(cx));
        Ok(())
    })
}

/// `attributes(x) <- value`: the attributes replaced by the named list's
/// (`dim` first, so that `dimnames` finds it); NULL takes them all away.
// R: attrib.c:do_attributesgets
pub fn do_attributesgets(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::by_name("attributes<-").unwrap().check_arity(cx, call, args)?;
    let (object, attrs) = (args.root(0), args.root(1));
    if !matches!(attrs.bind(cx).sexptype(), NILSXP | VECSXP) {
        return Err(cx.error("attributes must be a list or NULL"));
    }
    let nattrs = xlength(attrs.bind(cx));
    cx.scope(|cx, s| {
        let names = if nattrs > 0 {
            let Some(n) = super::names(attrs.bind(cx)) else {
                return Err(cx.error("attributes must be named"));
            };
            let n = s.root(cx, n);
            let unnamed = {
                let nv = n.bind(cx).as_vector::<Char<'_>>().unwrap();
                (1..nattrs).find(|&i| nv.get(i).bytes().is_empty())
            };
            if let Some(i) = unnamed {
                return Err(cx.error(&format!("all attributes must have names [{} does not]", i + 1)));
            }
            Some(n)
        } else {
            None
        };
        let obj = if object.bind(cx).sexptype() == NILSXP {
            if nattrs == 0 && attrs.bind(cx).sexptype() == NILSXP {
                out.set(cx, cx.nil());
                return Ok(());
            }
            let o = s.out(cx);
            let r = o.root();
            cx.build(o, |cx| Ok(cx.alloc_vector::<Ref<'_>>(0)?.finish().into()))?;
            r
        } else {
            let r = s.root(cx, object.bind(cx));
            let (shared, referenced) = (r.bind(cx).maybe_shared(), r.bind(cx).maybe_referenced());
            let outside = !call.bind(cx).is_assignment_call();
            if shared || (referenced && nattrs > 0) || (outside && referenced) {
                let o = s.out(cx);
                let d = o.root();
                cx.shallow_duplicate(r, o)?;
                r.replace_from(cx, d);
            }
            r
        };
        // empty the existing attribute list
        if obj.bind(cx).sexptype() == LISTSXP {
            remove_named(cx, obj, b"names")?;
        }
        // SET_ATTRIB(object, R_NilValue), SET_OBJECT(object, 0) and, with
        // no attributes to set, UNSET_S4_OBJECT: each written only when it
        // changes something (a symbol is a shared constant)
        if has_attrib(obj.bind(cx)) {
            let nil = s.root(cx, cx.nil());
            cx.set_attrib_list(obj, nil);
        }
        if obj.bind(cx).is_object() {
            cx.set_object_bit(obj, false);
        }
        if nattrs == 0 && obj.bind(cx).is_s4() {
            cx.set_s4_bit(obj, false);
        }
        if let Some(names) = names {
            let elt = |cx: &mut Cx, i: usize| {
                let View::List(l) = attrs.bind(cx).view() else { unreachable!("robigo: checked to be a list") };
                s.root(cx, l.get(i))
            };
            let dim = {
                let nv = names.bind(cx).as_vector::<Char<'_>>().unwrap();
                (0..nattrs).find(|&i| nv.get(i).bytes() == b"dim")
            };
            if let Some(i) = dim {
                let v = elt(cx, i);
                set_attrib(cx, obj, b"dim", v)?;
            }
            for i in (0..nattrs).filter(|&i| Some(i) != dim) {
                let nm = translated(cx, names, i)?;
                let v = elt(cx, i);
                set_attrib(cx, obj, &nm, v)?;
            }
        }
        out.set(cx, obj.bind(cx));
        Ok(())
    })
}

/// `oldClass(x) <- value`: the class attribute set as `classgets` does,
/// the S4 bit cleared.
// R: attrib.c:do_classgets
pub fn do_classgets(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::by_name("oldClass<-").unwrap().check_arity(cx, call, args)?;
    cx.scope(|cx, s| {
        let obj = writable(cx, s, call, args.root(0), Copy::Setter)?;
        let value = generic_length_zero(cx, s, args.root(1))?;
        if obj.bind(cx).is_s4() {
            cx.set_s4_bit(obj, false);
        }
        set_attrib(cx, obj, b"class", value)?;
        out.set(cx, obj.bind(cx));
        Ok(())
    })
}

/// `if (length(v) == 0) v = R_NilValue`, rooted.
fn generic_length_zero<'s>(cx: &mut Cx, s: Scope<'s>, v: Root<'_>) -> RResult<Root<'s>> {
    match crate::builtin::length(cx, v.bind(cx)) {
        Some(0) => Ok(s.root(cx, cx.nil())),
        Some(_) => Ok(s.root(cx, v.bind(cx))),
        None => Err(not_yet(cx, None, "the length of this environment")),
    }
}

/// `class(x) <- value`.
// R: coerce.c:R_do_set_class
pub fn do_set_class(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::by_name("class<-").unwrap().check_arity(cx, call, args)?;
    crate::util::check1arg(cx, args, call, "x")?;
    cx.scope(|cx, s| {
        let obj = writable(cx, s, call, args.root(0), Copy::Setter)?;
        let r = set_class(cx, s, call, obj, args.root(1))?;
        out.set(cx, r.bind(cx));
        Ok(())
    })
}

/// coerce.c's `classTable`: the basic classes, their types, and whether
/// assigning one coerces the object.
const CLASS_TABLE: &[(&[u8], u8, bool)] = &[
    (b"logical", LGLSXP, true),
    (b"integer", INTSXP, true),
    (b"double", REALSXP, true),
    (b"raw", RAWSXP, true),
    (b"complex", CPLXSXP, true),
    (b"character", STRSXP, true),
    (b"expression", EXPRSXP, true),
    (b"list", VECSXP, true),
    (b"environment", ENVSXP, false),
    (b"char", CHARSXP, true),
    (b"externalptr", EXTPTRSXP, false),
    (b"weakref", WEAKREFSXP, false),
    (b"name", SYMSXP, false),
];

/// `do_unsetS4(obj, newClass)`: the S4 bit cleared, with GNU R's warning.
// R: coerce.c:do_unsetS4
fn unset_s4(cx: &mut Cx, obj: Root<'_>, value: Root<'_>) -> RResult<()> {
    cx.set_s4_bit(obj, false);
    let msg = {
        let v = value.bind(cx);
        match v.as_vector::<Char<'_>>() {
            None => "Setting class(x) to NULL;   result will no longer be an S4 object".to_owned(),
            Some(c) if c.len() > 1 => format!(
                "Setting class(x) to multiple strings (\"{}\", \"{}\", ...); result will no longer be an S4 object",
                String::from_utf8_lossy(c.get(0).bytes()),
                String::from_utf8_lossy(c.get(1).bytes())
            ),
            Some(c) => format!(
                "Setting class(x) to \"{}\" sets attribute to NULL; result will no longer be an S4 object",
                String::from_utf8_lossy(c.get(0).bytes())
            ),
        }
    };
    cx.warning(&msg)
}

/// `R_set_class(obj, value, call)`: the object with the class `value`: a
/// basic class (`"numeric"`, `"matrix"`, `"array"`, a type's name) clears
/// the attribute and may coerce the object (a new one); any other value
/// is set as the class attribute.
// R: coerce.c:R_set_class
fn set_class<'s>(cx: &mut Cx, s: Scope<'s>, call: Root<'_>, obj: Root<'s>, value: Root<'_>) -> RResult<Root<'s>> {
    let nil = s.root(cx, cx.nil());
    let Some(len) = crate::builtin::length(cx, value.bind(cx)) else {
        return Err(not_yet(cx, Some(call), "'class<-' with an environment robigo cannot list"));
    };
    if len == 0 {
        set_attrib(cx, obj, b"class", value)?;
        if obj.bind(cx).is_s4() {
            unset_s4(cx, obj, value)?;
        }
        return Ok(obj);
    }
    let value = if value.bind(cx).sexptype() != STRSXP {
        let o = s.out(cx);
        let dup = o.root();
        cx.duplicate(value, o)?;
        let o = s.out(cx);
        let v = o.root();
        coerce_vector(cx, dup, STRSXP, o)?;
        if xlength(v.bind(cx)) == 0 {
            return Err(cx.error("invalid replacement object to be a class string"));
        }
        v
    } else {
        s.root(cx, value.bind(cx))
    };
    let (first, n) = {
        let v = value.bind(cx).as_vector::<Char<'_>>().unwrap();
        (v.get(0).bytes().to_vec(), v.len())
    };
    let ndim = len_of(get_value(obj.bind(cx), b"dim"));
    if first == b"matrix" {
        if ndim != 2 {
            let msg = format!(
                "invalid to set the class to matrix unless the dimension attribute is of length 2 (was {ndim})"
            );
            return Err(cx.error(&msg));
        }
        let second_array = n == 2 && value.bind(cx).as_vector::<Char<'_>>().unwrap().get(1).bytes() == b"array";
        if n == 1 || second_array {
            set_attrib(cx, obj, b"class", nil)?;
            if obj.bind(cx).is_s4() {
                unset_s4(cx, obj, value)?;
            }
            return Ok(obj);
        }
    }
    if n > 1 {
        set_attrib(cx, obj, b"class", value)?;
        if obj.bind(cx).is_s4() {
            unset_s4(cx, obj, value)?;
        }
        return Ok(obj);
    }
    if let Some(&(_, ty, can_change)) = CLASS_TABLE.iter().find(|(c, _, _)| *c == &first[..]) {
        set_attrib(cx, obj, b"class", nil)?;
        if obj.bind(cx).is_s4() {
            unset_s4(cx, obj, value)?;
        }
        if can_change {
            let o = s.out(cx);
            let r = o.root();
            let w = cx.build_with(o, |cx| crate::coerce::builtins::ascommon(cx, call, obj.bind(cx), ty, false))?;
            w.signal(cx)?;
            return Ok(r);
        }
        let t = obj.bind(cx).sexptype();
        if ty != t {
            let msg = format!(
                "\"{}\" can only be set as the class if the object has this type; found \"{}\"",
                String::from_utf8_lossy(&first),
                type_to_char(obj.bind(cx))
            );
            return Err(cx.error(&msg));
        }
        return Ok(obj);
    }
    match &first[..] {
        b"numeric" => {
            set_attrib(cx, obj, b"class", nil)?;
            if obj.bind(cx).is_s4() {
                unset_s4(cx, obj, value)?;
            }
            if matches!(obj.bind(cx).sexptype(), INTSXP | REALSXP) {
                return Ok(obj);
            }
            let o = s.out(cx);
            let r = o.root();
            coerce_vector(cx, obj, REALSXP, o)?;
            Ok(r)
        }
        b"array" => {
            if ndim == 0 {
                return Err(cx.error("cannot set class to \"array\" unless the dimension attribute has length > 0"));
            }
            set_attrib(cx, obj, b"class", nil)?;
            if obj.bind(cx).is_s4() {
                cx.set_s4_bit(obj, false);
            }
            Ok(obj)
        }
        _ => {
            set_attrib(cx, obj, b"class", value)?;
            Ok(obj)
        }
    }
}

/// `length(v)` of an attribute's value (NULL: 0).
fn len_of(v: Option<Ref<'_>>) -> usize {
    v.map_or(0, xlength)
}

/// `unclass(x)`: an object without its class attribute.
// R: objects.c:do_unclass
pub fn do_unclass(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::by_name("unclass").unwrap().check_arity(cx, call, args)?;
    crate::util::check1arg(cx, args, call, "x")?;
    let x = args.root(0);
    if !x.bind(cx).is_object() {
        out.set(cx, x.bind(cx));
        return Ok(());
    }
    match x.bind(cx).sexptype() {
        ENVSXP => return Err(cx.error_call(Some(call), "cannot unclass an environment")),
        EXTPTRSXP => return Err(cx.error_call(Some(call), "cannot unclass an external pointer")),
        _ => {}
    }
    cx.scope(|cx, s| {
        let obj = writable(cx, s, call, x, Copy::Referenced)?;
        remove_named(cx, obj, b"class")?;
        out.set(cx, obj.bind(cx));
        Ok(())
    })
}

const NAMES_GETS: Generic = Generic::builtin("names<-");
const DIM_GETS: Generic = Generic::builtin("dim<-");
const DIMNAMES_GETS: Generic = Generic::builtin("dimnames<-");
const LEVELS_GETS: Generic = Generic::builtin("levels<-");

/// `names(x) <- value`: an internal generic; a value other than NULL or a
/// character vector without attributes goes through `as.character` (R's,
/// which dispatches) first.
// R: attrib.c:do_namesgets
pub fn do_namesgets(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::by_name("names<-").unwrap().check_arity(cx, call, args)?;
    let Dispatched::Default(out) = cx.try_dispatch(&NAMES_GETS, call, args, env, out)? else { return Ok(()) };
    let (x0, val) = (args.root(0), args.root(1));
    // removing names that do not exist: no copy
    if val.bind(cx).sexptype() == NILSXP && !has_names(x0.bind(cx)) {
        out.set(cx, x0.bind(cx));
        return Ok(());
    }
    cx.scope(|cx, s| {
        let x = writable(cx, s, call, x0, Copy::Setter)?;
        if x.bind(cx).sexptype() == OBJSXP {
            let o = s.out(cx);
            let k = o.root();
            cx.build(o, |cx| super::data_class(cx, x.bind(cx)))?;
            let klass = translated(cx, k, 0)?;
            let msg =
                format!("invalid to use names()<- on an S4 object of class '{}'", String::from_utf8_lossy(&klass));
            return Err(cx.error(&msg));
        }
        let v = val.bind(cx);
        let names = if v.sexptype() != NILSXP && !(v.sexptype() == STRSXP && !has_attrib(v)) {
            as_character(cx, s, val, env)?
        } else {
            s.root(cx, v)
        };
        if names.bind(cx).sexptype() == NILSXP && one_dimensional(x.bind(cx)) {
            set_attrib(cx, x, b"dimnames", names)?;
        } else {
            set_attrib(cx, x, b"names", names)?;
        }
        out.set(cx, x.bind(cx));
        Ok(())
    })
}

/// `eval(as.character(value), env)` (R's, which dispatches), and then
/// `SETCADR(call, R_NilValue)`: the call no longer counts a reference to the
/// value.
// R: attrib.c:do_namesgets
fn as_character<'s>(cx: &mut Cx, s: Scope<'s>, v: Root<'_>, env: Root<'_>) -> RResult<Root<'s>> {
    let call = s.capture(cx, |cx, out| {
        cx.build(out, |cx| {
            let f = cx.install(b"as.character")?;
            let a = cx.cons(v.bind(cx), cx.nil())?;
            cx.lcons(f.into(), a)
        })
    })?;
    let r = s.capture(cx, |cx, out| cx.eval(call, env, out))?;
    let cell = s.root(cx, call.bind(cx).as_cons().unwrap().cdr());
    let nil = s.root(cx, cx.nil());
    cx.set_car(cell, nil);
    Ok(r)
}

/// Whether `getAttrib(x, R_NamesSymbol)` is not NULL: a pairlist's or
/// call's tags, a one-dimensional array's dimnames, the attribute.
fn has_names(x: Ref<'_>) -> bool {
    if matches!(x.sexptype(), LISTSXP | LANGSXP | DOTSXP) {
        let mut c = x.as_cons();
        while let Some(cell) = c {
            if cell.tag().as_sym().is_some() {
                return true;
            }
            c = cell.cdr().as_cons();
        }
    }
    super::names(x).is_some()
}

/// `dim(x) <- value`: an internal generic; the dim set (dropping the
/// dimnames), then the names removed. Removing a dim from an object with
/// neither dim nor names copies nothing.
// R: attrib.c:do_dimgets
pub fn do_dimgets(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::by_name("dim<-").unwrap().check_arity(cx, call, args)?;
    let Dispatched::Default(out) = dispatch_or_eval(cx, &DIM_GETS, call, args, env, out)? else { return Ok(()) };
    let (x0, val) = (args.root(0), args.root(1));
    if val.bind(cx).sexptype() == NILSXP
        && !super::attrs(x0.bind(cx)).any(|a| matches!(a.tag.name().bytes(), b"dim" | b"names"))
    {
        out.set(cx, x0.bind(cx));
        return Ok(());
    }
    cx.scope(|cx, s| {
        let x = writable(cx, s, call, x0, Copy::Setter)?;
        set_attrib(cx, x, b"dim", val)?;
        let nil = s.root(cx, cx.nil());
        set_attrib(cx, x, b"names", nil)?;
        out.set(cx, x.bind(cx));
        Ok(())
    })
}

/// `dimnames(x) <- value`: an internal generic over `dimnamesgets`.
// R: attrib.c:do_dimnamesgets
pub fn do_dimnamesgets(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::by_name("dimnames<-").unwrap().check_arity(cx, call, args)?;
    let Dispatched::Default(out) = dispatch_or_eval(cx, &DIMNAMES_GETS, call, args, env, out)? else {
        return Ok(());
    };
    cx.scope(|cx, s| {
        let x = writable(cx, s, call, args.root(0), Copy::Setter)?;
        set_attrib(cx, x, b"dimnames", args.root(1))?;
        out.set(cx, x.bind(cx));
        Ok(())
    })
}

/// `levels(x) <- value`: an internal generic; the levels must not repeat.
// R: attrib.c:do_levelsgets
pub fn do_levelsgets(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::by_name("levels<-").unwrap().check_arity(cx, call, args)?;
    let Dispatched::Default(out) = dispatch_or_eval(cx, &LEVELS_GETS, call, args, env, out)? else {
        return Ok(());
    };
    let val = args.root(1);
    if val.bind(cx).sexptype() != NILSXP {
        let d = crate::unique::any_duplicated_of(cx, val)?;
        if d > 0 {
            return Err(cx.error_call(Some(call), &format!("factor level [{d}] is duplicated")));
        }
    }
    cx.scope(|cx, s| {
        let x = writable(cx, s, call, args.root(0), Copy::SetterDeep)?;
        set_attrib(cx, x, b"levels", val)?;
        out.set(cx, x.bind(cx));
        Ok(())
    })
}

/// `.Internal(comment(x))`.
// R: attrib.c:do_comment
pub fn do_comment(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::lookup("comment", true).unwrap().check_arity(cx, call, args)?;
    let v = get_value(args.get(cx, 0), b"comment").unwrap_or(cx.nil());
    out.set(cx, v);
    Ok(())
}

/// `.Internal("comment<-"(x, value))`: an empty value removes the comment.
// R: attrib.c:do_commentgets
pub fn do_commentgets(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::lookup("comment<-", true).unwrap().check_arity(cx, call, args)?;
    cx.scope(|cx, s| {
        let x = writable(cx, s, call, args.root(0), Copy::SharedDeep)?;
        let value = generic_length_zero(cx, s, args.root(1))?;
        set_attrib(cx, x, b"comment", value)?;
        out.set(cx, x.bind(cx));
        Ok(())
    })
}
