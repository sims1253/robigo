//! `Cx::duplicate` of pairlists and calls, against `duplicate1`'s result
//! (duplicate.c): a new spine with the same tags and the head's type; each
//! car duplicated unless `duplicate1` shares its type (symbols, NULL,
//! environments, promises, ...); each cell's attributes copied deeply.
//! A copy whose cars are all shared and whose cells carry no attributes
//! skips the per-cell pass (S3y P1): these cases pin that its result is the
//! same as the full pass's on both sides of that test.

use robigo_rtcore::types::*;
use robigo_rtcore::{Cx, OwnedIsolate, Ref, Root};

fn sym<'g>(cx: &'g Cx, name: &[u8]) -> Ref<'g> {
    cx.install(name).unwrap().into()
}

/// The cells of a pairlist or call, from its head.
fn cells<'g>(l: Ref<'g>) -> Vec<Ref<'g>> {
    let mut v = Vec::new();
    let mut c = l.as_cons();
    while let Some(cell) = c {
        v.push(cell.as_ref());
        c = cell.cdr().as_cons();
    }
    v
}

/// `duplicate(x)` into a new root of `s`.
fn dup<'s>(cx: &mut Cx, s: robigo_rtcore::Scope<'s>, x: Root<'_>) -> Root<'s> {
    let out = s.out(cx);
    let d = out.root();
    cx.duplicate(x, out).unwrap();
    d
}

/// The spine of `d` is new, with `x`'s tags, and each cell but the head is
/// referenced once, by the cell before it (the copy's link state).
fn check_spine(x: Ref<'_>, d: Ref<'_>) {
    let (xc, dc) = (cells(x), cells(d));
    assert_eq!(xc.len(), dc.len());
    for (i, (a, b)) in xc.iter().zip(&dc).enumerate() {
        assert_ne!(a, b, "cell {i} must be a new cell, not the source's");
        let (ac, bc) = (a.as_cons().unwrap(), b.as_cons().unwrap());
        assert_eq!(ac.tag(), bc.tag(), "cell {i}'s tag");
        if i > 0 {
            assert_eq!(b.sexptype(), LISTSXP, "the copy's cell {i} is a pairlist cell");
            assert_eq!(b.refcnt(), 1, "the copy's cell {i} is referenced by the cell before it");
        }
    }
}

#[test]
fn a_call_of_symbols_is_a_new_spine_with_the_same_cars() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        // f(a, n = b)
        let tail = cx.cons_tagged(sym(cx, b"b"), cx.nil(), cx.install(b"n").unwrap()).unwrap();
        let mid = cx.cons(sym(cx, b"a"), tail).unwrap();
        let call = s.root(cx, cx.lcons(sym(cx, b"f"), mid).unwrap());
        let before: Vec<u16> = cells(call.bind(cx)).iter().map(|c| c.refcnt()).collect();
        let d = dup(cx, s, call);
        let (x, y) = (call.bind(cx), d.bind(cx));
        assert_eq!(y.sexptype(), LANGSXP, "the head keeps the call's type");
        check_spine(x, y);
        for (a, b) in cells(x).iter().zip(cells(y)) {
            assert_eq!(a.as_cons().unwrap().car(), b.as_cons().unwrap().car(), "a symbol is shared");
        }
        let after: Vec<u16> = cells(x).iter().map(|c| c.refcnt()).collect();
        assert_eq!(before, after, "the source's cells keep their counts");
    });
}

#[test]
fn a_pairlist_of_symbols_stays_a_pairlist() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let tail = cx.cons(cx.nil(), cx.nil()).unwrap();
        let l = s.root(cx, cx.cons(sym(cx, b"x"), tail).unwrap());
        let d = dup(cx, s, l);
        let (x, y) = (l.bind(cx), d.bind(cx));
        assert_eq!(y.sexptype(), LISTSXP);
        check_spine(x, y);
        assert_eq!(cells(y)[1].as_cons().unwrap().car(), cx.nil(), "NULL is shared");
    });
}

#[test]
fn a_call_with_a_call_and_a_vector_among_its_cars_copies_them() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        // f(g(x), 1)
        let one = cx.alloc_copy(&[1.0]).unwrap().finish().as_ref();
        let inner_args = cx.cons(sym(cx, b"x"), cx.nil()).unwrap();
        let inner = cx.lcons(sym(cx, b"g"), inner_args).unwrap();
        let tail = cx.cons(one, cx.nil()).unwrap();
        let mid = cx.cons(inner, tail).unwrap();
        let call = s.root(cx, cx.lcons(sym(cx, b"f"), mid).unwrap());
        let d = dup(cx, s, call);
        let (x, y) = (call.bind(cx), d.bind(cx));
        check_spine(x, y);
        let (xc, yc) = (cells(x), cells(y));
        let (xi, yi) = (xc[1].as_cons().unwrap().car(), yc[1].as_cons().unwrap().car());
        assert_ne!(xi, yi, "the inner call must be duplicated, not shared");
        assert_eq!(yi.sexptype(), LANGSXP);
        check_spine(xi, yi);
        let (xv, yv) = (xc[2].as_cons().unwrap().car(), yc[2].as_cons().unwrap().car());
        assert_ne!(xv, yv, "the vector must be duplicated, not shared");
        assert_eq!(yv.as_vector::<f64>().unwrap().as_slice(), Some(&[1.0][..]));
    });
}

/// `attr` cells hold one attribute, `name = value`, `value` a new vector.
fn with_attrib(cx: &Cx, cell: Ref<'_>, name: &[u8], value: f64) {
    let v = cx.alloc_copy(&[value]).unwrap().finish().as_ref();
    let a = cx.cons_tagged(v, cx.nil(), cx.install(name).unwrap()).unwrap();
    // SAFETY: `a` is a fresh cell; `cell` a live cell of the list being built.
    unsafe { cell.sexp().set_attrib(a.sexp()) };
}

fn attrib_value<'g>(cell: Ref<'g>) -> Ref<'g> {
    cell.attrib().unwrap().as_cons().unwrap().car()
}

#[test]
fn attributes_on_a_calls_head_are_copied_deeply() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        // f(a), its head carrying an attribute: the cars are symbols, so
        // only the attribute makes the per-cell pass run
        let args = cx.cons(sym(cx, b"a"), cx.nil()).unwrap();
        let call = s.root(cx, cx.lcons(sym(cx, b"f"), args).unwrap());
        with_attrib(cx, call.bind(cx), b"w", 2.0);
        let d = dup(cx, s, call);
        let (x, y) = (call.bind(cx), d.bind(cx));
        check_spine(x, y);
        assert_ne!(x.attrib().unwrap(), y.attrib().unwrap(), "the attribute cell must be new, not shared");
        let (xa, ya) = (attrib_value(x), attrib_value(y));
        assert_ne!(xa, ya, "the attribute's value must be duplicated, not shared");
        assert_eq!(ya.as_vector::<f64>().unwrap().as_slice(), Some(&[2.0][..]));
        assert_eq!(y.attrib().unwrap().as_cons().unwrap().tag(), x.attrib().unwrap().as_cons().unwrap().tag());
    });
}

#[test]
fn attributes_on_a_later_cell_are_copied_deeply() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        // a pairlist of symbols whose second cell carries an attribute
        let tail = cx.cons(sym(cx, b"b"), cx.nil()).unwrap();
        with_attrib(cx, tail, b"w", 3.0);
        let l = s.root(cx, cx.cons(sym(cx, b"a"), tail).unwrap());
        let d = dup(cx, s, l);
        let (x, y) = (l.bind(cx), d.bind(cx));
        check_spine(x, y);
        let (xt, yt) = (cells(x)[1], cells(y)[1]);
        let (xa, ya) = (attrib_value(xt), attrib_value(yt));
        assert_ne!(xa, ya, "the later cell's attribute value must be duplicated, not shared");
        assert_eq!(ya.as_vector::<f64>().unwrap().as_slice(), Some(&[3.0][..]));
        assert_eq!(cells(y)[0].attrib().map(|a| a.sexptype()), Some(NILSXP), "the head has none");
    });
}

#[test]
fn a_shared_car_gains_the_copys_reference() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        // an environment is shared by duplicate1, and counted: the copy's
        // cell references it, as the source's does
        let e = s.root(cx, cx.new_env(cx.base_env(), false, 0).unwrap());
        let tail = cx.cons(e.bind(cx), cx.nil()).unwrap();
        let l = s.root(cx, cx.cons(sym(cx, b"a"), tail).unwrap());
        assert_eq!(e.bind(cx).refcnt(), 1, "the source's cell");
        let d = dup(cx, s, l);
        check_spine(l.bind(cx), d.bind(cx));
        assert_eq!(cells(d.bind(cx))[1].as_cons().unwrap().car(), e.bind(cx), "shared, not copied");
        assert_eq!(e.bind(cx).refcnt(), 2, "and the copy's cell");
    });
}

#[test]
fn a_shared_promise_car_is_kept_and_counted() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        // a promise is shared by duplicate1 (not forced, not copied), and
        // counted like the environment above
        let p = s.root(cx, cx.mk_promise(sym(cx, b"a"), cx.base_env()).unwrap());
        let tail = cx.cons(p.bind(cx), cx.nil()).unwrap();
        let l = s.root(cx, cx.cons(sym(cx, b"x"), tail).unwrap());
        assert_eq!(p.bind(cx).refcnt(), 1, "the source's cell");
        let d = dup(cx, s, l);
        check_spine(l.bind(cx), d.bind(cx));
        assert_eq!(cells(d.bind(cx))[1].as_cons().unwrap().car(), p.bind(cx), "the promise must be shared, not copied");
        assert_eq!(p.bind(cx).refcnt(), 2, "and the copy's cell");
    });
}
