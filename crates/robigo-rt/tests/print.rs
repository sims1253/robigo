//! Printing and deparsing (`robigo_rt::print`, `robigo_rt::deparse`) of
//! values the base subset cannot build yet: named vectors, raw vectors,
//! pairlists, attributes, one-dimensional arrays, `print.default`'s
//! settings and `deparse()`'s controls.
//!
//! `print-oracle/cases.R` runs in GNU R 4.6.2 after `prelude.R`, which
//! binds `c2`, `structure2`, `pairlist2` and `raw2` to base's `c`,
//! `structure`, `pairlist` and `as.raw(c(...))` (`record.sh` records
//! `cases.Rout`); here the same script runs with those four as test
//! builtins, and its output must be GNU R's.
//!
//! The base sources are the pinned GNU R 4.6.2 tree (`scripts/r-src.sh`).

#![allow(unsafe_code)]

use robigo_rt::repl::run_console;
use robigo_rtbuiltins::attrib::{Attr, Attrs};
use robigo_rtcore::types::*;
use robigo_rtcore::{
    Args, Char, Console, Cx, FreshVec, Lgl, NeedGc, Out, OwnedIsolate, RResult, Rcomplex, Ref, Root, View,
};

fn session() -> OwnedIsolate {
    let dir = robigo_rt::base::source_dir().unwrap_or_else(|e| panic!("{e}"));
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    robigo_rt::install_with(cx, Some(robigo_rt::Profile::Eval));
    robigo_rt::base::load(cx, &dir).unwrap_or_else(|e| panic!("{e}"));
    robigo_rt::define_builtin(cx, "c2", c2, 1, -1);
    robigo_rt::define_builtin(cx, "structure2", structure2, 1, -1);
    robigo_rt::define_builtin(cx, "pairlist2", pairlist2, 1, -1);
    robigo_rt::define_builtin(cx, "raw2", raw2, 1, -1);
    robigo_rt::define_builtin(cx, "extptr2", extptr2, 1, 1);
    robigo_rt::define_builtin(cx, "weakref2", weakref2, 1, 0);
    iso
}

/// Run `src` as a script: (stdout, stderr, ran to the end).
fn run(cx: &mut Cx, src: &str) -> (String, String, bool) {
    let (c, out, err) = Console::capture();
    let old = cx.replace_console(c);
    let ok = run_console(cx, src.as_bytes());
    cx.replace_console(old);
    (String::from_utf8(out.take()).unwrap(), String::from_utf8(err.take()).unwrap(), ok)
}

/// The order `c()` coerces to: logical, integer, double, complex,
/// character.
fn rank(t: u8) -> u8 {
    match t {
        LGLSXP => 0,
        INTSXP => 1,
        REALSXP => 2,
        CPLXSXP => 3,
        STRSXP => 4,
        _ => panic!("c2: an atomic vector of a basic type, not type {t}"),
    }
}

/// One element as a double (NA stays NA).
fn real_at(x: Ref<'_>, i: usize) -> f64 {
    match x.view() {
        View::Lgl(v) => match v.get(i).0 {
            i32::MIN => robigo_text::NA_REAL,
            b => b as f64,
        },
        View::Int(v) => match v.get(i) {
            i32::MIN => robigo_text::NA_REAL,
            n => n as f64,
        },
        View::Real(v) => v.get(i),
        _ => unreachable!(),
    }
}

/// `c2(...)`: `c()` of atomic vectors of the basic types (no names).
fn c2(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.build(out, |cx| {
        let xs: Vec<Ref<'_>> = (0..args.len()).map(|i| args.get(cx, i)).collect();
        let t = xs.iter().map(|x| rank(x.sexptype())).max().unwrap_or(0);
        let all: Vec<(Ref<'_>, usize)> = xs.iter().flat_map(|&x| (0..x.len().unwrap()).map(move |i| (x, i))).collect();
        Ok(match t {
            0 => cx
                .alloc_copy(&all.iter().map(|&(x, i)| x.as_vector::<Lgl>().unwrap().get(i)).collect::<Vec<_>>())?
                .finish()
                .into(),
            1 => cx
                .alloc_copy(
                    &all.iter()
                        .map(|&(x, i)| match x.view() {
                            View::Lgl(v) => v.get(i).0,
                            View::Int(v) => v.get(i),
                            _ => unreachable!(),
                        })
                        .collect::<Vec<_>>(),
                )?
                .finish()
                .into(),
            2 => cx.alloc_copy(&all.iter().map(|&(x, i)| real_at(x, i)).collect::<Vec<_>>())?.finish().into(),
            3 => {
                let z: Vec<Rcomplex> = all
                    .iter()
                    .map(|&(x, i)| match x.view() {
                        View::Cplx(v) => v.get(i),
                        // coerceVector: a logical or integer NA is NA+0i
                        _ => Rcomplex { r: real_at(x, i), i: 0.0 },
                    })
                    .collect();
                cx.alloc_copy(&z)?.finish().into()
            }
            _ => {
                let mut v = cx.alloc_vector::<Char<'_>>(all.len())?;
                for (k, &(x, i)) in all.iter().enumerate() {
                    let c = match x.view() {
                        View::Str(s) => s.get(i),
                        View::Lgl(l) if l.get(i).0 == i32::MIN => cx.na_string(),
                        _ => panic!("c2: strings and NA only"),
                    };
                    v.set(k, c);
                }
                v.finish().into()
            }
        })
    })
}

/// A copy of the atomic vector or list `x` with `attrs` applied.
fn copy_with<'g>(cx: &'g Cx, x: Ref<'g>, attrs: &Attrs<'g>) -> Result<Ref<'g>, NeedGc> {
    fn plain<'g, T: robigo_rtcore::Plain + robigo_rtcore::Elt<'g>>(
        cx: &'g Cx,
        x: Ref<'g>,
        attrs: &Attrs<'g>,
    ) -> Result<Ref<'g>, NeedGc> {
        let mut v: FreshVec<'g, T> = cx.alloc_copy(x.as_vector::<T>().unwrap().as_slice().unwrap())?;
        attrs.apply(cx, &mut v)?;
        Ok(v.finish().into())
    }
    match x.view() {
        View::Lgl(_) => plain::<Lgl>(cx, x, attrs),
        View::Int(_) => plain::<i32>(cx, x, attrs),
        View::Real(_) => plain::<f64>(cx, x, attrs),
        View::Cplx(_) => plain::<Rcomplex>(cx, x, attrs),
        View::Raw(_) => plain::<u8>(cx, x, attrs),
        View::Str(s) => {
            let mut v = cx.alloc_vector::<Char<'_>>(s.len())?;
            for i in 0..s.len() {
                v.set(i, s.get(i));
            }
            attrs.apply(cx, &mut v)?;
            Ok(v.finish().into())
        }
        View::List(l) => {
            let mut v = cx.alloc_vector::<Ref<'_>>(l.len())?;
            for i in 0..l.len() {
                v.set(i, l.get(i));
            }
            attrs.apply(cx, &mut v)?;
            Ok(v.finish().into())
        }
        _ => panic!("structure2: an atomic vector or a list"),
    }
}

/// `structure2(x, name = value, ...)`: `x` with the attributes added in
/// order (`class` sets OBJECT).
fn structure2(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.build(out, |cx| {
        let x = args.get(cx, 0);
        let mut a = Attrs::new();
        a.copy_all(x);
        for i in 1..args.len() {
            let tag = args.name(cx, i).expect("structure2: named attributes");
            if tag.name().bytes() == b"class" {
                a.set_class(tag, Some(args.get(cx, i)));
            } else {
                a.install(Attr { tag, value: args.get(cx, i) });
            }
        }
        copy_with(cx, x, &a)
    })
}

/// `pairlist2(...)`: the arguments as a pairlist, tagged by their names.
fn pairlist2(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.build(out, |cx| {
        let mut l = cx.nil();
        for i in (0..args.len()).rev() {
            l = match args.name(cx, i) {
                Some(t) => cx.cons_tagged(args.get(cx, i), l, t)?,
                None => cx.cons(args.get(cx, i), l)?,
            };
        }
        Ok(l)
    })
}

/// `raw2(...)`: `as.raw(c(...))` of numbers in 0..=255.
fn raw2(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.build(out, |cx| {
        let mut b = Vec::new();
        for i in 0..args.len() {
            let x = args.get(cx, i);
            for k in 0..x.len().unwrap() {
                b.push(real_at(x, k) as u8);
            }
        }
        Ok(cx.alloc_copy(&b)?.finish().into())
    })
}

/// `extptr2(address)`: an external pointer to the address (a double).
fn extptr2(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let addr = real_at(args.get(cx, 0), 0) as usize;
    cx.build(out, |cx| {
        let nil = cx.nil().sexp();
        // SAFETY: an external pointer's car is its address, which the heap
        // never traces (EXTPTRSXP); its protected value and tag are
        // R_NilValue, immortal. The result is fresh and `build` roots it.
        let x = unsafe { cx.alloc_node_raw(EXTPTRSXP, robigo_rtcore::Sexp(addr as *mut u8), nil, nil)? };
        // SAFETY: fresh, of this heap.
        Ok(unsafe { Ref::from_sexp(x, cx) })
    })
}

/// `weakref2()`: a weak reference (key, value and finalizer NULL).
fn weakref2(cx: &mut Cx, _call: Root<'_>, _args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.build(out, |cx| {
        let x = cx.alloc_vector_raw(WEAKREFSXP, 4)?;
        // SAFETY: fresh, of this heap.
        Ok(unsafe { Ref::from_sexp(x, cx) })
    })
}

#[test]
fn the_cases_print_as_gnu_r_4_6_2_prints_them() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/print-oracle");
    let src = std::fs::read_to_string(dir.join("cases.R")).unwrap();
    let want_out = std::fs::read_to_string(dir.join("cases.Rout")).unwrap();
    let want_err = std::fs::read_to_string(dir.join("cases.Rerr")).unwrap();
    let mut iso = session();
    let (out, err, ok) = run(iso.cx(), &src);
    assert!(ok, "{err}");
    let (out_lines, want_lines): (Vec<_>, Vec<_>) = (out.lines().collect(), want_out.lines().collect());
    for (i, (a, b)) in out_lines.iter().zip(&want_lines).enumerate() {
        assert_eq!(a, b, "stdout line {}", i + 1);
    }
    assert_eq!(out, want_out);
    assert_eq!(err, want_err);
}

#[test]
fn pointers_and_weak_references() {
    let mut iso = session();
    let (out, err, ok) = run(iso.cx(), "extptr2(0)\nextptr2(4660)\nweakref2()\nlist(extptr2(0), weakref2())\n");
    assert!(ok, "{err}");
    // print.c: "<pointer: %p>" (glibc's %p of NULL is "(nil)"), "<weak reference>"
    assert_eq!(
        out,
        "<pointer: (nil)>\n<pointer: 0x1234>\n<weak reference>\n[[1]]\n<pointer: (nil)>\n\n[[2]]\n<weak reference>\n\n"
    );
}

#[test]
fn what_is_not_ported_says_so() {
    let mut iso = session();
    for (src, msg) in [
        ("structure2(1:4, dim = c2(2L, 2L))\n", "robigo: printing a matrix (printMatrix) is not implemented yet"),
        ("structure2(1:8, dim = c2(2L, 2L, 2L))\n", "robigo: printing an array (printArray) is not implemented yet"),
        ("list(sum)\n", "robigo: printing the primitive 'sum'"),
        ("f <- function() 1\nf\n", "robigo: the primitive 'UseMethod' is not implemented yet"),
    ] {
        let (_, err, ok) = run(iso.cx(), src);
        assert!(!ok && err.contains(msg), "{src}: {err}");
    }
}
