//! options.c: `.Options` (a tagged pairlist, the value of base's
//! `.Options`), `options(...)` and `getOption(x)`, and the C globals the
//! options mirror (`R_ShowErrorMessages`, `R_WarnLength`, the traceback
//! switches, `R_Expressions`, `R_warn_partial_match_args`), which the
//! condition system reads.
//!
//! A new session's `.Options` is `InitOptions`' list followed by what
//! Rscript's system profile (`library/profile/Common.R`) sets in a
//! non-interactive session with `R_DEFAULT_PACKAGES=NULL`. The options
//! `Rprofile.unix` derives from the installation (`papersize`, `browser`,
//! `pager`, ...) are not set, so `options()` with no arguments, which lists
//! them all, is "not implemented yet".

#![allow(unsafe_code)]

use robigo_heap::types::*;
use robigo_rtcore::{Args, Char, Cx, Encoding, Lgl, Out, RResult, Ref, Root, Scope};

/// `R_MIN_WIDTH_OPT` .. `R_MAX_EXPRESSIONS_OPT`.
const MIN_WIDTH: i32 = 10;
const MAX_WIDTH: i32 = 10000;
const MIN_DIGITS: i32 = 1;
const MAX_DIGITS: i32 = 22;
const MIN_EXPRESSIONS: i32 = 25;
const MAX_EXPRESSIONS: i32 = 500000;

/// An option's initial value.
enum Init {
    Str(&'static str),
    Int(i32),
    Lgl(i32),
    Strs(&'static [&'static str]),
}

/// `InitOptions`' list, then Common.R's additions (in order).
// R: options.c:InitOptions; library/profile/Common.R
const INITIAL: &[(&str, Init)] = &[
    ("prompt", Init::Str("> ")),
    ("continue", Init::Str("+ ")),
    ("expressions", Init::Int(5000)),
    ("width", Init::Int(80)),
    ("deparse.cutoff", Init::Int(60)),
    ("digits", Init::Int(7)),
    // Rscript: --no-echo
    ("echo", Init::Lgl(0)),
    ("quiet", Init::Lgl(1)),
    ("verbose", Init::Lgl(0)),
    ("check.bounds", Init::Lgl(0)),
    // Common.R: keep.source = interactive()
    ("keep.source", Init::Lgl(0)),
    ("keep.source.pkgs", Init::Lgl(0)),
    ("keep.parse.data", Init::Lgl(1)),
    ("keep.parse.data.pkgs", Init::Lgl(0)),
    ("warning.length", Init::Int(1000)),
    ("nwarnings", Init::Int(50)),
    ("OutDec", Init::Str(".")),
    ("CBoundsCheck", Init::Lgl(0)),
    ("matprod", Init::Str("default")),
    ("PCRE_study", Init::Lgl(0)),
    ("PCRE_use_JIT", Init::Lgl(1)),
    ("PCRE_limit_recursion", Init::Lgl(i32::MIN)),
    ("max.contour.segments", Init::Int(25000)),
    ("warnPartialMatchDollar", Init::Lgl(0)),
    ("warnPartialMatchArgs", Init::Lgl(0)),
    ("warnPartialMatchAttr", Init::Lgl(0)),
    ("showWarnCalls", Init::Lgl(0)),
    // Common.R: if (!interactive()) options(showErrorCalls = TRUE)
    ("showErrorCalls", Init::Lgl(1)),
    ("showNCalls", Init::Int(50)),
    ("browserNLdisabled", Init::Lgl(0)),
    ("rl_word_breaks", Init::Str(" \t\n\"\\'`><=;,|&{()}")),
    ("warn", Init::Int(0)),
    ("timeout", Init::Int(60)),
    ("encoding", Init::Str("native.enc")),
    ("show.error.messages", Init::Lgl(1)),
    ("scipen", Init::Int(0)),
    ("max.print", Init::Int(99999)),
    ("add.smooth", Init::Lgl(1)),
    ("catch.script.errors", Init::Lgl(0)),
    // R_DEFAULT_PACKAGES=NULL
    ("defaultPackages", Init::Strs(&[])),
];

/// The symbol `name`, rooted.
fn sym<'s>(cx: &mut Cx, s: Scope<'s>, name: &[u8]) -> RResult<Root<'s>> {
    let o = s.out(cx);
    let r = o.root();
    cx.build(o, |cx| Ok(cx.install(name)?.as_ref()))?;
    Ok(r)
}

/// Make a new session's `.Options` and bind it in base (`InitOptions`,
/// then Common.R's), and set the mirrors. Part of [`crate::install`].
pub(crate) fn init(cx: &mut Cx) {
    cx.scope(|cx, s| {
        let o = s.out(cx);
        let list = o.root();
        cx.build(o, |cx| {
            let mut l = cx.nil();
            for (name, v) in INITIAL.iter().rev() {
                let val: Ref<'_> = match v {
                    Init::Str(t) => {
                        let mut x = cx.alloc_vector::<Char<'_>>(1)?;
                        x.set(0, cx.mk_char(t.as_bytes(), Encoding::Native)?);
                        x.finish().into()
                    }
                    Init::Int(i) => cx.alloc_copy(&[*i])?.finish().into(),
                    Init::Lgl(b) => cx.alloc_copy(&[Lgl(*b)])?.finish().into(),
                    Init::Strs(v) => {
                        let mut x = cx.alloc_vector::<Char<'_>>(v.len())?;
                        for (i, t) in v.iter().enumerate() {
                            x.set(i, cx.mk_char(t.as_bytes(), Encoding::Native)?);
                        }
                        x.finish().into()
                    }
                };
                let tag = cx.install(name.as_bytes())?;
                l = cx.cons_tagged(val, l, tag)?;
            }
            Ok(l)
        })
        .expect("robigo: no room for the options");
        let osym = sym(cx, s, b".Options").expect("robigo: no room for the options");
        let base = s.root(cx, cx.base_env());
        cx.define_var(osym, list, base).expect("robigo: binding .Options");
    });
}

/// The `.Options` list (its value in base), rooted.
fn options_list<'s>(cx: &mut Cx, s: Scope<'s>) -> RResult<Root<'s>> {
    let osym = sym(cx, s, b".Options")?;
    let base = s.root(cx, cx.base_env());
    let o = s.out(cx);
    let r = o.root();
    cx.find_var_in_frame3(base, osym, true, o)?;
    let t = r.bind(cx).sexptype();
    if t != LISTSXP && t != NILSXP {
        return Err(cx.error("corrupted options list"));
    }
    Ok(r)
}

/// `FindTaggedItem(lst, tag)`: the cell tagged `tag`, or None; an error
/// for a NULL value.
fn find_tagged<'g>(_cx: &'g Cx, lst: Ref<'g>, tag: &[u8]) -> Result<Option<Ref<'g>>, String> {
    let mut l = lst;
    while let Some(c) = l.as_cons() {
        if c.tag().as_sym().is_some_and(|t| t.name().bytes() == tag) {
            if c.car().sexptype() == NILSXP {
                return Err(format!("option {} has NULL value", String::from_utf8_lossy(tag)));
            }
            return Ok(Some(l));
        }
        l = c.cdr();
    }
    Ok(None)
}

/// `GetOption1(install(name))`: the option's value, R_NilValue if unset.
// R: options.c:GetOption1
pub fn get_option1<'s>(cx: &mut Cx, s: Scope<'s>, name: &[u8]) -> RResult<Root<'s>> {
    let lst = options_list(cx, s)?;
    let found = find_tagged(cx, lst.bind(cx), name);
    match found {
        Err(m) => Err(cx.error(&m)),
        Ok(None) => Ok(s.root(cx, cx.nil())),
        Ok(Some(c)) => Ok(s.root(cx, c.as_cons().unwrap().car())),
    }
}

/// `SetOption(tag, value)`: the old value (R_NilValue if unset) written to
/// `old`; R_NilValue removes the option; a new one is appended.
// R: options.c:SetOption
fn set_option(cx: &mut Cx, name: &[u8], value: Root<'_>, old: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let lst = options_list(cx, s)?;
        let found = match find_tagged(cx, lst.bind(cx), name) {
            Ok(f) => f.map(|c| s.root(cx, c)),
            Err(m) => return Err(cx.error(&m)),
        };
        let nil = cx.nil().sexp();
        if value.bind(cx).sexptype() == NILSXP {
            // removal: the cell after `t` whose tag is the name
            let mut t = lst.bind(cx).sexp();
            // SAFETY: the rooted list's cells; SETCDR of one of them.
            unsafe {
                while t != nil {
                    let next = t.cdr();
                    if next != nil
                        && next.tag() != nil
                        && Ref::from_sexp(next.tag(), cx).as_sym().unwrap().name().bytes() == name
                    {
                        old.set(cx, Ref::from_sexp(next.car(), cx));
                        t.set_cdr(next.cdr());
                        return Ok(());
                    }
                    t = next;
                }
            }
            old.set(cx, cx.nil());
            return Ok(());
        }
        match found {
            Some(cell) => {
                // SAFETY: a cell of the rooted list; SETCAR with a rooted value.
                unsafe {
                    let c = cell.bind(cx).sexp();
                    old.set(cx, Ref::from_sexp(c.car(), cx));
                    c.set_car(value.bind(cx).sexp());
                }
            }
            None => {
                let tag = sym(cx, s, name)?;
                let o = s.out(cx);
                let cell = o.root();
                cx.build(o, |cx| cx.cons_tagged(value.bind(cx), cx.nil(), tag.bind(cx).as_sym().unwrap()))?;
                let l = lst.bind(cx).sexp();
                if l == nil {
                    let base = s.root(cx, cx.base_env());
                    let osym = sym(cx, s, b".Options")?;
                    cx.define_var(osym, cell, base)?;
                } else {
                    // SAFETY: the rooted list's last cell; SETCDR with the
                    // rooted new cell.
                    unsafe {
                        let mut t = l;
                        while t.cdr() != nil {
                            t = t.cdr();
                        }
                        t.set_cdr(cell.bind(cx).sexp());
                    }
                }
                old.set(cx, cx.nil());
            }
        }
        Ok(())
    })
}

/// `.Internal(getOption(x))`.
// R: options.c:do_getOption
pub fn do_getoption(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity(cx, call, args, "getOption", 1)?;
    let x = args.get(cx, 0);
    let Some(v) = x.as_vector::<Char<'_>>().filter(|v| x.sexptype() == STRSXP && v.len() == 1) else {
        return Err(cx.error("'x' must be a character string"));
    };
    let name = robigo_rtbuiltins::coerce::text(v.get(0)).bytes().to_vec();
    cx.scope(|cx, s| {
        let v = get_option1(cx, s, &name)?;
        // duplicate(): a shallow copy shares nothing GNU R's deep one
        // would not for the atomic values options hold
        cx.shallow_duplicate(v, out)
    })
}

fn check_arity(cx: &mut Cx, call: Root<'_>, args: Args<'_>, name: &str, n: usize) -> RResult<()> {
    if args.len() == n {
        return Ok(());
    }
    let k = args.len();
    let msg = format!("{k} argument{} passed to .Internal({name}) which requires {n}", if k == 1 { "" } else { "s" });
    Err(cx.error_call(Some(call), &msg))
}

/// `asInteger` of an option's value (GNU R's coercion; NA for others).
fn as_int(cx: &mut Cx, x: Root<'_>) -> RResult<i32> {
    robigo_rtbuiltins::util::as_integer(cx, x)
}

/// `check_TRUE_FALSE(arg, name)`.
fn check_true_false(cx: &mut Cx, x: Root<'_>, name: &str) -> RResult<bool> {
    let v = x.bind(cx);
    match v.as_vector::<Lgl>().filter(|l| v.sexptype() == LGLSXP && l.len() == 1) {
        Some(l) if l.get(0) != Lgl::NA => Ok(l.get(0).0 != 0),
        _ => Err(cx.error(&format!("invalid value for '{name}'"))),
    }
}

/// The value made by `ScalarInteger(k)` / `ScalarLogical(k)`, rooted.
fn scalar_int<'s>(cx: &mut Cx, s: Scope<'s>, k: i32) -> RResult<Root<'s>> {
    let o = s.out(cx);
    let r = o.root();
    cx.build(o, |cx| Ok(cx.alloc_copy(&[k])?.finish().into()))?;
    Ok(r)
}

fn scalar_lgl<'s>(cx: &mut Cx, s: Scope<'s>, k: i32) -> RResult<Root<'s>> {
    let o = s.out(cx);
    let r = o.root();
    cx.build(o, |cx| Ok(cx.alloc_copy(&[Lgl(k)])?.finish().into()))?;
    Ok(r)
}

/// `options(...)`: each `name = value` set (its old value returned), each
/// unnamed string queried; the result's visibility is a query's.
// R: options.c:do_options
pub fn do_options(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    if args.is_empty() {
        return Err(cx.error_call(
            Some(call),
            "robigo: options() with no arguments (the installation's options from Rprofile.unix) is not implemented yet",
        ));
    }
    cx.scope(|cx, s| {
        // the (name, value) pairs: the arguments, or the one list given
        let mut items: Vec<(Option<Vec<u8>>, Root<'_>)> = Vec::new();
        let first = args.get(cx, 0);
        if args.len() == 1 && args.name(cx, 0).is_none() && matches!(first.sexptype(), LISTSXP | VECSXP) {
            match first.sexptype() {
                VECSXP => {
                    let n = first.len().unwrap();
                    let names = robigo_rtbuiltins::attrib::get_value(first, b"names");
                    if n > 0 && names.and_then(|nm| nm.len()) != Some(n) {
                        return Err(cx.error("list argument has no valid names"));
                    }
                    for i in 0..n {
                        let nm = names.and_then(|nm| nm.as_vector::<Char<'_>>()).map(|v| v.get(i).bytes().to_vec());
                        let v = s.root(cx, first.as_vector::<Ref<'_>>().unwrap().get(i));
                        items.push((nm.filter(|n| !n.is_empty()), v));
                    }
                }
                _ => {
                    let mut c = first;
                    let mut cells = Vec::new();
                    while let Some(cell) = c.as_cons() {
                        cells.push((cell.tag().as_sym().map(|t| t.name().bytes().to_vec()), cell.car()));
                        c = cell.cdr();
                    }
                    for (nm, v) in cells {
                        let v = s.root(cx, v);
                        items.push((nm.filter(|n| !n.is_empty()), v));
                    }
                }
            }
        } else {
            for i in 0..args.len() {
                let nm = args.name(cx, i).map(|t| t.name().bytes().to_vec());
                items.push((nm.filter(|n| !n.is_empty()), args.root(i)));
            }
        }
        let n = items.len();
        let values: Vec<Root<'_>> = (0..n).map(|_| s.out(cx).root()).collect();
        let mut names: Vec<Vec<u8>> = vec![Vec::new(); n];
        let mut visible = false;
        for (i, (nm, argi)) in items.iter().enumerate() {
            let argi = *argi;
            match nm {
                Some(name) => {
                    names[i] = name.clone();
                    let slot = values[i];
                    set_one(cx, s, call, name, argi, slot)?;
                }
                None => {
                    // a query
                    let v = argi.bind(cx);
                    let Some(sv) = v.as_vector::<Char<'_>>().filter(|sv| v.sexptype() == STRSXP && !sv.is_empty())
                    else {
                        return Err(cx.error("invalid argument"));
                    };
                    let tag = robigo_rtbuiltins::coerce::text(sv.get(0)).bytes().to_vec();
                    let raw = sv.get(0).bytes().to_vec();
                    if tag == b"par.ask.default" {
                        return Err(cx.error("\"par.ask.default\" has been replaced by \"device.ask.default\""));
                    }
                    let cur = get_option1(cx, s, &tag)?;
                    let o = values[i].reset(cx);
                    cx.shallow_duplicate(cur, o)?;
                    names[i] = raw;
                    visible = true;
                }
            }
        }
        cx.build(out, |cx| {
            let mut v = cx.alloc_vector::<Ref<'_>>(n)?;
            for (i, r) in values.iter().enumerate() {
                v.set(i, r.bind(cx));
            }
            let mut nm = cx.alloc_vector::<Char<'_>>(n)?;
            for (i, b) in names.iter().enumerate() {
                nm.set(i, if b.is_empty() { cx.blank_string() } else { cx.mk_char(b, Encoding::Native)? });
            }
            let nsym = cx.install(b"names")?;
            let attrs = cx.cons_tagged(nm.finish().as_ref(), cx.nil(), nsym)?;
            v.set_attrib(attrs);
            Ok(v.finish().into())
        })?;
        cx.set_visible(visible);
        Ok(())
    })
}

/// The condition state, shared (an `Rc`): held across `&mut Cx` calls.
fn prims_cond(cx: &Cx) -> std::rc::Rc<crate::conditions::CondState> {
    crate::prims::state(cx).cond.clone()
}

/// One `name = value` of `options()`, its old value written to `slot`.
fn set_one(cx: &mut Cx, s: Scope<'_>, call: Root<'_>, name: &[u8], argi: Root<'_>, slot: Root<'_>) -> RResult<()> {
    let nm = String::from_utf8_lossy(name).into_owned();
    let invalid = |cx: &mut Cx| cx.error(&format!("invalid value for '{nm}'"));
    let o = slot.reset(cx);
    if argi.bind(cx).sexptype() == NILSXP {
        const MANDATORY: &[&str] = &[
            "prompt",
            "continue",
            "expressions",
            "width",
            "deparse.cutoff",
            "digits",
            "echo",
            "quiet",
            "verbose",
            "check.bounds",
            "keep.source",
            "keep.source.pkgs",
            "keep.parse.data",
            "keep.parse.data.pkgs",
            "warning.length",
            "nwarnings",
            "OutDec",
            "CBoundsCheck",
            "matprod",
            "PCRE_study",
            "PCRE_use_JIT",
            "PCRE_limit_recursion",
            "rl_word_breaks",
            "max.contour.segments",
            "warnPartialMatchDollar",
            "warnPartialMatchArgs",
            "warnPartialMatchAttr",
            "showWarnCalls",
            "showErrorCalls",
            "showNCalls",
            "browserNLdisabled",
            "warn",
            "max.print",
            "show.error.messages",
            "scipen",
        ];
        if MANDATORY.contains(&nm.as_str()) {
            return Err(cx.error(&format!("option '{nm}' cannot be deleted")));
        }
        return set_option(cx, name, argi, o);
    }
    let st = prims_cond(cx);
    match nm.as_str() {
        "width" | "digits" => {
            let k = as_int(cx, argi)?;
            let (lo, hi) = if nm == "width" { (MIN_WIDTH, MAX_WIDTH) } else { (MIN_DIGITS, MAX_DIGITS) };
            if k < lo || k > hi {
                return Err(cx.error(&format!("invalid '{nm}' parameter, allowed {lo}...{hi}")));
            }
            let v = scalar_int(cx, s, k)?;
            set_option(cx, name, v, o)
        }
        "deparse.cutoff" => {
            let k = as_int(cx, argi)?;
            let v = scalar_int(cx, s, k)?;
            set_option(cx, name, v, o)
        }
        "expressions" => {
            let k = as_int(cx, argi)?;
            if !(MIN_EXPRESSIONS..=MAX_EXPRESSIONS).contains(&k) {
                return Err(
                    cx.error(&format!("invalid '{nm}' parameter, allowed {MIN_EXPRESSIONS}...{MAX_EXPRESSIONS}"))
                );
            }
            cx.set_expressions(k);
            let v = scalar_int(cx, s, k)?;
            set_option(cx, name, v, o)
        }
        "warn" => {
            let a = argi.bind(cx);
            if !matches!(a.sexptype(), INTSXP | REALSXP | LGLSXP)
                || robigo_rtbuiltins::attrib::inherits(a, b"factor")
                || a.len() != Some(1)
            {
                return Err(invalid(cx));
            }
            let k = match a.as_vector::<f64>() {
                Some(d) => {
                    let d = d.get(0);
                    if d.is_nan() || d >= 2147483648.0 || d <= -2147483648.0 { i32::MIN } else { d as i32 }
                }
                None => as_int(cx, argi)?,
            };
            if k == i32::MIN {
                return Err(invalid(cx));
            }
            let v = scalar_int(cx, s, k)?;
            set_option(cx, name, v, o)
        }
        "warning.length" => {
            let k = as_int(cx, argi)?;
            if !(100..=8170).contains(&k) {
                return Err(invalid(cx));
            }
            st.warn_length.set(k as usize);
            set_option(cx, name, argi, o)
        }
        "warning.expression" => {
            if !matches!(argi.bind(cx).sexptype(), LANGSXP | EXPRSXP) {
                return Err(invalid(cx));
            }
            set_option(cx, name, argi, o)
        }
        "max.print" | "nwarnings" => {
            let k = as_int(cx, argi)?;
            if k < 1 {
                return Err(invalid(cx));
            }
            if nm == "nwarnings" {
                return Err(cx.error_call(Some(call), "robigo: options(nwarnings) is not implemented yet"));
            }
            let v = scalar_int(cx, s, k)?;
            set_option(cx, name, v, o)
        }
        "error" => {
            let t = argi.bind(cx).sexptype();
            let v = if matches!(t, CLOSXP | BUILTINSXP | SPECIALSXP) {
                // makeErrorCall(fun)
                let c = s.out(cx);
                let cr = c.root();
                cx.build(c, |cx| cx.lcons(argi.bind(cx), cx.nil()))?;
                cr
            } else if t != LANGSXP && t != EXPRSXP {
                return Err(invalid(cx));
            } else {
                argi
            };
            set_option(cx, name, v, o)
        }
        "show.error.messages" => {
            let b = check_true_false(cx, argi, &nm)?;
            st.show_error_messages.set(b);
            set_option(cx, name, argi, o)
        }
        "catch.script.errors"
        | "browserNLdisabled"
        | "CBoundsCheck"
        | "warnPartialMatchDollar"
        | "warnPartialMatchAttr" => {
            check_true_false(cx, argi, &nm)?;
            set_option(cx, name, argi, o)
        }
        "warnPartialMatchArgs" => {
            let b = check_true_false(cx, argi, &nm)?;
            cx.set_warn_partial_match_args(b);
            set_option(cx, name, argi, o)
        }
        "showWarnCalls" => {
            let b = check_true_false(cx, argi, &nm)?;
            st.show_warn_calls.set(b);
            set_option(cx, name, argi, o)
        }
        "showErrorCalls" => {
            let b = check_true_false(cx, argi, &nm)?;
            st.show_error_calls.set(b);
            set_option(cx, name, argi, o)
        }
        "showNCalls" => {
            let k = as_int(cx, argi)?;
            if !(30..=500).contains(&k) || argi.bind(cx).len() != Some(1) {
                return Err(invalid(cx));
            }
            st.n_show_calls.set(k as usize);
            let v = scalar_int(cx, s, k)?;
            set_option(cx, name, v, o)
        }
        "check.bounds" | "echo" | "quiet" | "verbose" | "keep.source" => {
            let a = argi.bind(cx);
            if a.sexptype() != LGLSXP || a.len() != Some(1) {
                return Err(invalid(cx));
            }
            let k = robigo_rtbuiltins::util::as_logical(cx, a);
            if nm != "check.bounds" && nm != "echo" && k == i32::MIN {
                return Err(cx.error_call(Some(call), "NA in coercion to boolean"));
            }
            let v = scalar_lgl(cx, s, k)?;
            set_option(cx, name, v, o)
        }
        "par.ask.default" => Err(cx.error("\"par.ask.default\" has been replaced by \"device.ask.default\"")),
        "editor"
        | "continue"
        | "prompt"
        | "contrasts"
        | "scipen"
        | "OutDec"
        | "max.contour.segments"
        | "rl_word_breaks"
        | "matprod"
        | "PCRE_study"
        | "PCRE_use_JIT"
        | "PCRE_limit_recursion"
        | "stringsAsFactors" => {
            let msg = format!("robigo: options({nm} = ...) is not implemented yet");
            Err(cx.error_call(Some(call), &msg))
        }
        _ => {
            let v = s.out(cx);
            let vr = v.root();
            cx.shallow_duplicate(argi, v)?;
            set_option(cx, name, vr, o)
        }
    }
}
