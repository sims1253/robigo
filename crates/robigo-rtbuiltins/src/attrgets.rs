//! attrib.c's attribute reads and writes beyond `names`: `attributes`,
//! `attributes<-`, `attr`, `oldClass<-` (`classgets`), coerce.c's `class<-`
//! (`R_set_class`) and objects.c's `unclass`; `setAttrib`'s dispatch to
//! the special names.
//!
//! Not here yet, each an error that says so: setting `dim`, `dimnames`,
//! `tsp`, `comment` or `row.names` (`dimgets`, ...), the implicit classes
//! `class<-` coerces to (`ascommon`), S4 objects, and `attr`'s partial
//! matching of names other than its exact one.

use crate::attrib;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Out, RResult, Ref, Root, Scope};

/// A symbol rooted in `s`.
fn sym<'s>(cx: &mut Cx, s: Scope<'s>, name: &[u8]) -> RResult<Root<'s>> {
    let o = s.out(cx);
    let r = o.root();
    cx.build(o, |cx| Ok(cx.install(name)?.as_ref()))?;
    Ok(r)
}

/// The object to write: `x` itself if the call may change it in place,
/// else a shallow copy (`MAYBE_SHARED(x) || (!IS_ASSIGNMENT_CALL(call) &&
/// MAYBE_REFERENCED(x))`).
fn writable<'s>(cx: &mut Cx, s: Scope<'s>, call: Root<'_>, x: Root<'_>) -> RResult<Root<'s>> {
    let v = x.bind(cx);
    if v.maybe_shared() || (!call.bind(cx).is_assignment_call() && v.maybe_referenced()) {
        let o = s.out(cx);
        let r = o.root();
        cx.shallow_duplicate(x, o)?;
        Ok(r)
    } else {
        Ok(s.root(cx, v))
    }
}

/// `classgets(vec, klass)`: a class vector installs the attribute and
/// sets OBJECT; NULL or an empty one removes both.
// R: attrib.c:classgets
fn classgets(cx: &mut Cx, s: Scope<'_>, vec: Root<'_>, klass: Root<'_>) -> RResult<()> {
    let kt = klass.bind(cx).sexptype();
    if kt != NILSXP && kt != STRSXP {
        return Err(cx.error("attempt to set invalid 'class' attribute"));
    }
    let csym = sym(cx, s, b"class")?;
    let k = klass.bind(cx);
    if k.len().unwrap_or(0) == 0 {
        cx.remove_attrib(vec, csym)?;
        cx.set_object_bit(vec, false);
        return Ok(());
    }
    if vec.bind(cx).sexptype() == NILSXP {
        return Err(cx.error("attempt to set an attribute on NULL"));
    }
    let isfactor = k.as_vector::<Char<'_>>().unwrap().as_slice().unwrap().iter().any(|c| c.bytes() == b"factor");
    if isfactor && vec.bind(cx).sexptype() != INTSXP {
        return Err(cx.error("adding class \"factor\" to an invalid object"));
    }
    cx.install_attrib(vec, csym, klass)?;
    cx.set_object_bit(vec, true);
    Ok(())
}

/// `setAttrib(vec, name, val)` on an object the caller may write: NULL
/// removes the attribute; `names` and `class` go through their own
/// setters; the other special names are not ported yet.
// R: attrib.c:setAttrib
pub fn set_attrib(cx: &mut Cx, s: Scope<'_>, vec: Root<'_>, name: &[u8], val: Root<'_>) -> RResult<()> {
    if val.bind(cx).sexptype() == NILSXP && name != b"class" {
        let n = sym(cx, s, name)?;
        return cx.remove_attrib(vec, n);
    }
    if vec.bind(cx).sexptype() == NILSXP {
        return Err(cx.error("attempt to set an attribute on NULL"));
    }
    match name {
        b"names" => crate::attrib::namesgets(cx, s, vec, val),
        b"class" => classgets(cx, s, vec, val),
        b"dim" | b"dimnames" | b"tsp" | b"comment" | b"row.names" => {
            let what = format!("setting the '{}' attribute", String::from_utf8_lossy(name));
            Err(crate::util::not_yet(cx, None, &what))
        }
        _ => {
            let n = sym(cx, s, name)?;
            cx.install_attrib(vec, n, val)
        }
    }
}

/// `attributes(x)`: a named list of the attributes (a pairlist's names
/// first), NULL for none.
// R: attrib.c:do_attributes, R_getAttributes
pub fn do_attributes(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("attributes", false).expect("robigo: a registry row").check_arity(cx, call, args)?;
    crate::is::check1arg(cx, args, call, "x")?;
    let x = args.get(cx, 0);
    let is_pair = matches!(x.sexptype(), LISTSXP | NILSXP);
    let pair_names = is_pair && {
        let mut c = x;
        let mut any = false;
        while let Some(cell) = c.as_cons() {
            any |= cell.tag().sexptype() != NILSXP;
            c = cell.cdr();
        }
        any
    };
    if attrib::attrs(x).next().is_none() && !pair_names {
        out.set(cx, cx.nil());
        return Ok(());
    }
    if pair_names {
        return Err(crate::util::not_yet(cx, Some(call), "attributes() of a tagged pairlist"));
    }
    if attrib::get(x, b"row.names").is_some() {
        return Err(crate::util::not_yet(cx, Some(call), "attributes() with row.names (getAttrib's compact form)"));
    }
    cx.build(out, |cx| {
        let x = args.get(cx, 0);
        let items: Vec<attrib::Attr<'_>> = attrib::attrs(x).collect();
        let mut v = cx.alloc_vector::<Ref<'_>>(items.len())?;
        let mut nm = cx.alloc_vector::<Char<'_>>(items.len())?;
        for (i, a) in items.iter().enumerate() {
            v.set(i, a.value);
            nm.set(i, a.tag.name());
        }
        let nsym = cx.install(b"names")?;
        let al = cx.cons_tagged(nm.finish().as_ref(), cx.nil(), nsym)?;
        v.set_attrib(al);
        Ok(v.finish().into())
    })
}

/// `attributes(x) <- value`: every attribute replaced by the named list's
/// (`dim` first); OBJECT cleared and set again by a class.
// R: attrib.c:do_attributesgets
pub fn do_attributesgets(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("attributes<-", false).expect("robigo: a registry row").check_arity(cx, call, args)?;
    let attrs = args.get(cx, 1);
    if !matches!(attrs.sexptype(), VECSXP | NILSXP) {
        return Err(cx.error("attributes must be a list or NULL"));
    }
    let nattrs = if attrs.sexptype() == NILSXP { 0 } else { attrs.len().unwrap() };
    let names: Vec<Vec<u8>> = if nattrs > 0 {
        let Some(nm) = attrib::get_value(attrs, b"names") else {
            return Err(cx.error("attributes must be named"));
        };
        let nm = nm.as_vector::<Char<'_>>().unwrap();
        for i in 1..nattrs {
            if nm.get(i).bytes().is_empty() {
                return Err(cx.error(&format!("all attributes must have names [{} does not]", i + 1)));
            }
        }
        (0..nattrs).map(|i| nm.get(i).bytes().to_vec()).collect()
    } else {
        Vec::new()
    };
    cx.scope(|cx, s| {
        let object = if args.get(cx, 0).sexptype() == NILSXP {
            if nattrs == 0 {
                out.set(cx, cx.nil());
                return Ok(());
            }
            let o = s.out(cx);
            let r = o.root();
            cx.build(o, |cx| Ok(cx.alloc_vector::<Ref<'_>>(0)?.finish().into()))?;
            r
        } else {
            let v = args.get(cx, 0);
            if v.maybe_shared()
                || (v.maybe_referenced() && nattrs > 0)
                || (!call.bind(cx).is_assignment_call() && v.maybe_referenced())
            {
                // R_shallow_duplicate_attr: a shallow copy
                let o = s.out(cx);
                let r = o.root();
                cx.shallow_duplicate(args.root(0), o)?;
                r
            } else {
                args.root(0)
            }
        };
        if matches!(object.bind(cx).sexptype(), LISTSXP) {
            return Err(crate::util::not_yet(cx, Some(call), "'attributes<-' on a pairlist (its names are tags)"));
        }
        let nil = s.root(cx, cx.nil());
        cx.set_attrib_list(object, nil);
        cx.set_object_bit(object, false);
        if nattrs == 0 {
            cx.set_s4_bit(object, false);
        }
        // dim first, then the rest in order
        let vals: Vec<Root<'_>> =
            (0..nattrs).map(|i| s.root(cx, args.get(cx, 1).as_vector::<Ref<'_>>().unwrap().get(i))).collect();
        let i0 = names.iter().position(|n| n == b"dim");
        if let Some(i) = i0 {
            set_attrib(cx, s, object, b"dim", vals[i])?;
        }
        for (i, n) in names.iter().enumerate() {
            if Some(i) == i0 {
                continue;
            }
            set_attrib(cx, s, object, n, vals[i])?;
        }
        out.set(cx, object.bind(cx));
        Ok(())
    })
}

/// `attr(x, which, exact = FALSE)`: the attribute named `which` (exactly,
/// or by a unique partial match unless `exact`).
// R: attrib.c:do_attr
pub fn do_attr(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let n = args.len();
    if !(2..=3).contains(&n) {
        return Err(cx.error_call(Some(call), "either 2 or 3 arguments are required"));
    }
    // (argument matching against (x, which, exact): positional here)
    let which = args.get(cx, 1);
    let Some(w) = which.as_vector::<Char<'_>>().filter(|v| which.sexptype() == STRSXP && v.len() == 1) else {
        return Err(cx.error("exactly one attribute 'which' must be given"));
    };
    let name = w.get(0).bytes().to_vec();
    let exact = if n == 3 {
        let e = crate::util::as_logical(cx, args.get(cx, 2));
        e != i32::MIN && e != 0
    } else {
        false
    };
    if name == b"names" || name == b"row.names" {
        return Err(crate::util::not_yet(cx, Some(call), "attr() of 'names' or 'row.names' (getAttrib's special forms)"));
    }
    let x = args.get(cx, 0);
    if let Some(v) = attrib::get_value(x, &name) {
        out.set(cx, v);
        return Ok(());
    }
    if exact {
        out.set(cx, cx.nil());
        return Ok(());
    }
    // a unique partial match
    let partial: Vec<Ref<'_>> =
        attrib::attrs(x).filter(|a| a.tag.name().bytes().starts_with(&name)).map(|a| a.value).collect();
    match partial.len() {
        1 => {
            if cx.warn_partial_match_args() {
                return Err(crate::util::not_yet(cx, Some(call), "attr()'s partial-match warning (warnPartialMatchAttr)"));
            }
            let v = partial[0];
            out.set(cx, v);
            Ok(())
        }
        _ => {
            out.set(cx, cx.nil());
            Ok(())
        }
    }
}

/// `oldClass(x) <- value`.
// R: attrib.c:do_classgets
pub fn do_classgets(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("oldClass<-", false).expect("robigo: a registry row").check_arity(cx, call, args)?;
    cx.scope(|cx, s| {
        let x = writable(cx, s, call, args.root(0))?;
        let v = if crate::array::r_length(cx, args.get(cx, 1)) == 0 { s.root(cx, cx.nil()) } else { args.root(1) };
        if x.bind(cx).is_s4() {
            cx.set_s4_bit(x, false);
        }
        set_attrib(cx, s, x, b"class", v)?;
        out.set(cx, x.bind(cx));
        Ok(())
    })
}

/// `class(x) <- value` (`R_set_class`): a class vector of several strings,
/// or one that is not an implicit class, set as the attribute; NULL
/// removes it.
// R: coerce.c:R_do_set_class, R_set_class
pub fn do_set_class(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("class<-", false).expect("robigo: a registry row").check_arity(cx, call, args)?;
    crate::is::check1arg(cx, args, call, "x")?;
    cx.scope(|cx, s| {
        let obj = writable(cx, s, call, args.root(0))?;
        if obj.bind(cx).is_s4() {
            return Err(crate::util::not_yet(cx, Some(call), "'class<-' on an S4 object"));
        }
        let value = args.root(1);
        if crate::array::r_length(cx, value.bind(cx)) == 0 {
            set_attrib(cx, s, obj, b"class", value)?;
            out.set(cx, obj.bind(cx));
            return Ok(());
        }
        if value.bind(cx).sexptype() != STRSXP {
            return Err(crate::util::not_yet(cx, Some(call), "'class<-' with a value that is not a character vector"));
        }
        let first = value.bind(cx).as_vector::<Char<'_>>().unwrap().get(0).bytes().to_vec();
        let nv = value.bind(cx).len().unwrap();
        const IMPLICIT: &[&[u8]] = &[
            b"matrix",
            b"array",
            b"numeric",
            b"logical",
            b"integer",
            b"double",
            b"raw",
            b"complex",
            b"character",
            b"expression",
            b"list",
            b"environment",
            b"char",
            b"externalptr",
            b"weakref",
            b"name",
            b"NULL",
            b"function",
        ];
        if (nv == 1 && IMPLICIT.contains(&&first[..])) || (first == b"matrix") {
            return Err(crate::util::not_yet(cx, Some(call), "'class<-' to an implicit class (ascommon)"));
        }
        set_attrib(cx, s, obj, b"class", value)?;
        out.set(cx, obj.bind(cx));
        Ok(())
    })
}

/// `unclass(x)`: the object without its class attribute (a shallow copy if
/// it is referenced).
// R: objects.c:do_unclass
pub fn do_unclass(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("unclass", false).expect("robigo: a registry row").check_arity(cx, call, args)?;
    crate::is::check1arg(cx, args, call, "x")?;
    let x = args.get(cx, 0);
    if !x.is_object() {
        out.set(cx, x);
        return Ok(());
    }
    match x.sexptype() {
        ENVSXP => return Err(cx.error_call(Some(call), "cannot unclass an environment")),
        EXTPTRSXP => return Err(cx.error_call(Some(call), "cannot unclass an external pointer")),
        _ => {}
    }
    cx.scope(|cx, s| {
        let v = args.get(cx, 0);
        let obj = if v.maybe_referenced() {
            // R_shallow_duplicate_attr
            let o = s.out(cx);
            let r = o.root();
            cx.shallow_duplicate(args.root(0), o)?;
            r
        } else {
            args.root(0)
        };
        let nil = s.root(cx, cx.nil());
        set_attrib(cx, s, obj, b"class", nil)?;
        out.set(cx, obj.bind(cx));
        Ok(())
    })
}
