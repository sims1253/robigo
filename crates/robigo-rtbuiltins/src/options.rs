//! options.c: the option list `.Options` (a tagged pairlist bound in base),
//! `.Internal(options(...))` and `.Internal(getOption(x))`.
//!
//! [`init_options`] makes the list GNU R has when an Rscript's code starts:
//! `InitOptions`' entries, then those the system profile (`Common.R`)
//! sets with values that do not depend on the installation. The entries
//! `Rprofile.unix` derives from `R_HOME` and the platform (`papersize`,
//! `printcmd`, `pager`, `pdfviewer`, `browser`, ...) are not set.
//!
//! GNU R changes `.Options` in place (`SETCAR`, `SETCDR`); here an
//! assignment rebinds `.Options` to an updated copy of the list, which
//! differs only for code that holds the old list itself.

use crate::attrib::{Attr, Attrs};
use crate::registry;
use crate::util::not_yet;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Encoding, Lgl, NeedGc, Out, RResult, Ref, Root, Sym, View};

/// An option's initial value.
enum Init {
    Str(&'static [u8]),
    Int(i32),
    Lgl(i32),
    /// `options(defaultPackages = ...)` from `R_DEFAULT_PACKAGES`.
    DefaultPackages,
}

/// `InitOptions()`'s list, then `Common.R`'s settings, in order.
// R: options.c:InitOptions, library/profile/Common.R
const INIT: &[(&str, Init)] = &[
    ("prompt", Init::Str(b"> ")),
    ("continue", Init::Str(b"+ ")),
    ("expressions", Init::Int(5000)),
    ("width", Init::Int(80)),
    ("deparse.cutoff", Init::Int(60)),
    ("digits", Init::Int(7)),
    ("echo", Init::Lgl(0)),
    ("quiet", Init::Lgl(1)),
    ("verbose", Init::Lgl(0)),
    ("check.bounds", Init::Lgl(0)),
    ("keep.source", Init::Lgl(0)),
    ("keep.source.pkgs", Init::Lgl(0)),
    ("keep.parse.data", Init::Lgl(1)),
    ("keep.parse.data.pkgs", Init::Lgl(0)),
    ("warning.length", Init::Int(1000)),
    ("nwarnings", Init::Int(50)),
    ("OutDec", Init::Str(b".")),
    ("CBoundsCheck", Init::Lgl(0)),
    ("matprod", Init::Str(b"default")),
    ("PCRE_study", Init::Lgl(0)),
    ("PCRE_use_JIT", Init::Lgl(1)),
    ("PCRE_limit_recursion", Init::Lgl(i32::MIN)),
    ("max.contour.segments", Init::Int(25000)),
    ("warnPartialMatchDollar", Init::Lgl(0)),
    ("warnPartialMatchArgs", Init::Lgl(0)),
    ("warnPartialMatchAttr", Init::Lgl(0)),
    ("showWarnCalls", Init::Lgl(0)),
    ("showErrorCalls", Init::Lgl(1)),
    ("showNCalls", Init::Int(50)),
    ("browserNLdisabled", Init::Lgl(0)),
    ("rl_word_breaks", Init::Str(b" \t\n\"\\'`><=;,|&{()}")),
    // Common.R
    ("warn", Init::Int(0)),
    ("timeout", Init::Int(60)),
    ("encoding", Init::Str(b"native.enc")),
    ("show.error.messages", Init::Lgl(1)),
    ("scipen", Init::Int(0)),
    ("max.print", Init::Int(99999)),
    ("add.smooth", Init::Lgl(1)),
    ("catch.script.errors", Init::Lgl(0)),
    ("defaultPackages", Init::DefaultPackages),
];

/// The packages of `R_DEFAULT_PACKAGES` as `Common.R` reads it.
fn default_packages() -> Vec<String> {
    match std::env::var("R_DEFAULT_PACKAGES").as_deref() {
        Err(_) | Ok("") => ["datasets", "utils", "grDevices", "graphics", "stats", "methods"].map(String::from).to_vec(),
        Ok("NULL") => Vec::new(),
        Ok(s) => s.split(',').map(|p| p.trim_start_matches([' ', '\t']).to_string()).collect(),
    }
}

/// The `.Options` symbol.
fn options_sym(cx: &Cx) -> Result<Sym<'_>, NeedGc> {
    cx.install(b".Options")
}

/// Bind `.Options` in base to the initial option list.
pub fn init_options(cx: &mut Cx) -> RResult<()> {
    cx.scope(|cx, s| {
        let l = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let mut head = cx.nil();
                for (name, init) in INIT.iter().rev() {
                    let v: Ref<'_> = match init {
                        Init::Str(b) => crate::coerce::mk_string(cx, b)?,
                        Init::Int(i) => cx.alloc_copy(&[*i])?.finish().into(),
                        Init::Lgl(i) => cx.alloc_copy(&[Lgl(*i)])?.finish().into(),
                        Init::DefaultPackages => {
                            let p = default_packages();
                            let mut v = cx.alloc_vector::<Char<'_>>(p.len())?;
                            for (i, n) in p.iter().enumerate() {
                                v.set(i, cx.mk_char(n.as_bytes(), Encoding::Native)?);
                            }
                            v.finish().into()
                        }
                    };
                    head = cx.cons_tagged(v, head, cx.install(name.as_bytes())?)?;
                }
                Ok(head)
            })
        })?;
        let sym = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(options_sym(cx)?.into())))?;
        let base = s.root(cx, cx.base_env());
        cx.define_var(sym, l, base)
    })
}

/// The option list (`SYMVALUE(.Options)`), rooted in `s`.
fn options_list<'s>(cx: &mut Cx, s: robigo_rtcore::Scope<'s>) -> RResult<Root<'s>> {
    let sym = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(options_sym(cx)?.into())))?;
    let base = s.root(cx, cx.base_env());
    let l = s.capture(cx, |cx, o| cx.find_var_in_frame3(base, sym, true, o))?;
    let v = l.bind(cx);
    if !(v.sexptype() == LISTSXP || v.sexptype() == NILSXP) {
        return Err(cx.error("corrupted options list"));
    }
    Ok(l)
}

/// `FindTaggedItem(lst, tag)`'s value: None when there is no such option.
// R: options.c:FindTaggedItem, GetOption1
fn find<'g>(cx: &'g Cx, l: Ref<'g>, name: &[u8]) -> Result<Option<Ref<'g>>, String> {
    let _ = cx;
    let mut c = l.as_cons();
    while let Some(cell) = c {
        if cell.tag().as_sym().is_some_and(|t| t.name().bytes() == name) {
            if cell.car().sexptype() == NILSXP {
                return Err(format!("option {} has NULL value", String::from_utf8_lossy(name)));
            }
            return Ok(Some(cell.car()));
        }
        c = cell.cdr().as_cons();
    }
    Ok(None)
}

/// `GetOption1(install(name))` for code below `rt`: the option's value, or
/// NULL.
pub fn get_option<'s>(cx: &mut Cx, s: robigo_rtcore::Scope<'s>, name: &[u8]) -> RResult<Root<'s>> {
    let l = options_list(cx, s)?;
    let r = find(cx, l.bind(cx), name).map(|v| v.unwrap_or(cx.nil()));
    match r {
        Ok(v) => Ok(s.root(cx, v)),
        Err(m) => Err(cx.error(&m)),
    }
}

/// `.Internal(getOption(x))`: the option's value (GNU R a duplicate of
/// it; the value is shared and copied on its next modification here).
// R: options.c:do_getOption
pub fn do_get_option(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::lookup("getOption", true).unwrap().check_arity(cx, call, args)?;
    let name = match args.get(cx, 0).as_vector::<Char<'_>>() {
        Some(v) if v.len() == 1 => v.get(0).bytes().to_vec(),
        _ => return Err(cx.error("'x' must be a character string")),
    };
    cx.scope(|cx, s| {
        let v = get_option(cx, s, &name)?;
        out.set(cx, v.bind(cx));
        Ok(())
    })
}

/// The list with `name` set to `value` (`SetOption`: replaced in place,
/// or appended), or removed for None; and the old value.
// R: options.c:SetOption
fn set_option<'g>(cx: &'g Cx, l: Ref<'g>, name: &[u8], value: Option<Ref<'g>>) -> Result<(Ref<'g>, Ref<'g>), NeedGc> {
    let mut items: Vec<(Sym<'g>, Ref<'g>)> = Vec::new();
    let mut c = l.as_cons();
    while let Some(cell) = c {
        items.push((cell.tag().as_sym().expect("robigo: a tagged option"), cell.car()));
        c = cell.cdr().as_cons();
    }
    let pos = items.iter().position(|(t, _)| t.name().bytes() == name);
    let old = pos.map_or(cx.nil(), |p| items[p].1);
    match (pos, value) {
        (Some(p), None) => {
            items.remove(p);
        }
        (None, None) => return Ok((l, old)),
        (Some(p), Some(v)) => items[p].1 = v,
        (None, Some(v)) => items.push((cx.install(name)?, v)),
    }
    let mut head = cx.nil();
    for (t, v) in items.iter().rev() {
        head = cx.cons_tagged(*v, head, *t)?;
    }
    Ok((head, old))
}

/// GNU R's `mandatory[]`: options that cannot be removed.
const MANDATORY: &[&str] = &[
    "prompt", "continue", "expressions", "width", "deparse.cutoff", "digits", "echo", "quiet", "verbose",
    "check.bounds", "keep.source", "keep.source.pkgs", "keep.parse.data", "keep.parse.data.pkgs",
    "warning.length", "nwarnings", "OutDec", "CBoundsCheck", "matprod", "PCRE_study", "PCRE_use_JIT",
    "PCRE_limit_recursion", "rl_word_breaks", "max.contour.segments", "warnPartialMatchDollar",
    "warnPartialMatchArgs", "warnPartialMatchAttr", "showWarnCalls", "showErrorCalls", "showNCalls",
    "browserNLdisabled", "warn", "max.print", "show.error.messages", "scipen",
];

/// `.Internal(options(...))`: the sorted list with no arguments; else, per
/// argument, a query (an unnamed string) or an assignment (`name =
/// value`), the old values returned, invisibly unless something was
/// queried.
// R: options.c:do_options
pub fn do_options(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let l = options_list(cx, s)?;
        if args.is_empty() {
            return Err(not_yet(cx, Some(call), "options() with no arguments (orderVector1 of the names: ICU collation)"));
        }
        // a single unnamed list (or pairlist) argument is the arguments
        let mut items: Vec<(Option<Vec<u8>>, Root<'_>)> = Vec::new();
        let single = args.len() == 1 && args.name(cx, 0).is_none() && {
            let t = args.get(cx, 0).sexptype();
            matches!(t, LISTSXP | VECSXP | EXPRSXP | NILSXP)
        };
        if single {
            let a = args.get(cx, 0);
            match a.view() {
                View::List(v) | View::Expr(v) => {
                    let n = v.len();
                    let names = crate::attrib::names(a).and_then(|x| x.as_vector::<Char<'_>>());
                    if n > 0 && names.is_none_or(|nm| nm.len() != n) {
                        return Err(cx.error("list argument has no valid names"));
                    }
                    let elts: Vec<(Option<Vec<u8>>, Ref<'_>)> =
                        (0..n).map(|i| (Some(names.unwrap().get(i).bytes().to_vec()), v.get(i))).collect();
                    for (nm, e) in elts {
                        let r = s.root(cx, e);
                        items.push((nm.filter(|b| !b.is_empty()), r));
                    }
                }
                _ => {
                    let mut c = a.as_cons();
                    let mut elts = Vec::new();
                    while let Some(cell) = c {
                        elts.push((cell.tag().as_sym().map(|t| t.name().bytes().to_vec()), cell.car()));
                        c = cell.cdr().as_cons();
                    }
                    for (nm, e) in elts {
                        let r = s.root(cx, e);
                        items.push((nm, r));
                    }
                }
            }
        } else {
            for i in 0..args.len() {
                items.push((args.name(cx, i).map(|t| t.name().bytes().to_vec()), args.root(i)));
            }
        }
        let n = items.len();
        let values: Vec<Root<'_>> = (0..n).map(|_| s.root(cx, cx.nil())).collect();
        let mut names: Vec<Vec<u8>> = vec![Vec::new(); n];
        let mut visible = false;
        for (i, (name, argi)) in items.iter().enumerate() {
            match name {
                Some(nm) => {
                    names[i] = nm.clone();
                    let v = check_option(cx, s, call, nm, *argi)?;
                    if v.is_none() && MANDATORY.contains(&std::str::from_utf8(nm).unwrap_or("")) {
                        let msg = format!("option '{}' cannot be deleted", String::from_utf8_lossy(nm));
                        return Err(cx.error(&msg));
                    }
                    let nl = s.out(cx);
                    let nlr = nl.root();
                    let old = s.out(cx);
                    let oldr = old.root();
                    cx.build(nl, |cx| {
                        let (newl, _) = set_option(cx, l.bind(cx), nm, v.map(|r| r.bind(cx)))?;
                        Ok(newl)
                    })?;
                    cx.build(old, |cx| {
                        let (_, o) = set_option(cx, l.bind(cx), nm, v.map(|r| r.bind(cx)))?;
                        Ok(o)
                    })?;
                    values[i].replace_from(cx, oldr);
                    l.replace_from(cx, nlr);
                    let sym = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(options_sym(cx)?.into())))?;
                    let base = s.root(cx, cx.base_env());
                    cx.define_var(sym, l, base)?;
                }
                None => {
                    let tag = match argi.bind(cx).as_vector::<Char<'_>>() {
                        Some(v) if !v.is_empty() => v.get(0),
                        _ => return Err(cx.error("invalid argument")),
                    };
                    if tag.bytes() == b"par.ask.default" {
                        return Err(cx.error("\"par.ask.default\" has been replaced by \"device.ask.default\""));
                    }
                    let tb = tag.bytes().to_vec();
                    let r = find(cx, l.bind(cx), &tb).map(|v| v.unwrap_or(cx.nil()));
                    let v = match r {
                        Ok(v) => s.root(cx, v),
                        Err(m) => return Err(cx.error(&m)),
                    };
                    values[i].replace_from(cx, v);
                    names[i] = tb;
                    visible = true;
                }
            }
        }
        cx.build(out, |cx| {
            let mut ans = cx.alloc_vector::<Ref<'_>>(n)?;
            for (i, v) in values.iter().enumerate() {
                ans.set(i, v.bind(cx));
            }
            let mut nm = cx.alloc_vector::<Char<'_>>(n)?;
            for (i, b) in names.iter().enumerate() {
                nm.set(i, cx.mk_char(b, Encoding::Native)?);
            }
            let mut a = Attrs::new();
            a.install(Attr { tag: cx.install(b"names")?, value: nm.finish().into() });
            a.apply(cx, &mut ans)?;
            Ok(ans.finish().into())
        })?;
        cx.set_visible(visible);
        Ok(())
    })
}

/// The value an assignment `name = argi` stores (GNU R's per-option checks
/// and normalizations), None for a removal (NULL); the options GNU R also
/// mirrors in C state are mirrored where robigo reads them.
fn check_option<'s>(
    cx: &mut Cx,
    s: robigo_rtcore::Scope<'s>,
    call: Root<'_>,
    name: &[u8],
    argi: Root<'_>,
) -> RResult<Option<Root<'s>>> {
    let a = argi.bind(cx);
    if a.sexptype() == NILSXP {
        return Ok(None);
    }
    let nm = String::from_utf8_lossy(name).into_owned();
    let invalid = |cx: &mut Cx| -> robigo_rtcore::RError { cx.error(&format!("invalid value for '{nm}'")) };
    let int = |cx: &mut Cx, s: robigo_rtcore::Scope<'s>, k: i32| -> RResult<Root<'s>> {
        s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.alloc_copy(&[k])?.finish().into())))
    };
    let lgl = |cx: &mut Cx, s: robigo_rtcore::Scope<'s>, k: i32| -> RResult<Root<'s>> {
        s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.alloc_copy(&[Lgl(k)])?.finish().into())))
    };
    let tf = |cx: &mut Cx| -> RResult<bool> {
        let a = argi.bind(cx);
        if a.sexptype() != LGLSXP || a.len() != Some(1) || crate::util::as_logical(cx, a) == i32::MIN {
            return Err(cx.error(&format!("invalid value for '{nm}'")));
        }
        Ok(crate::util::as_logical(cx, a) != 0)
    };
    let as_int = |cx: &mut Cx| crate::coerce::vector::as_integer_warn(cx, argi);
    Ok(Some(match name {
        b"width" => {
            let k = as_int(cx)?;
            if !(10..=10000).contains(&k) {
                return Err(cx.error(&format!("invalid '{nm}' parameter, allowed 10...10000")));
            }
            int(cx, s, k)?
        }
        b"deparse.cutoff" => {
            let k = as_int(cx)?;
            int(cx, s, k)?
        }
        b"digits" => {
            let k = as_int(cx)?;
            if !(0..=22).contains(&k) {
                return Err(cx.error(&format!("invalid '{nm}' parameter, allowed 0...22")));
            }
            int(cx, s, k)?
        }
        b"expressions" => {
            let k = as_int(cx)?;
            if !(25..=500000).contains(&k) {
                return Err(cx.error(&format!("invalid '{nm}' parameter, allowed 25...500000")));
            }
            int(cx, s, k)?
        }
        b"keep.source" | b"check.bounds" | b"echo" | b"quiet" | b"verbose" => {
            let a = argi.bind(cx);
            if a.sexptype() != LGLSXP || a.len() != Some(1) {
                return Err(invalid(cx));
            }
            let k = if matches!(name, b"check.bounds" | b"echo") {
                crate::util::as_logical(cx, a)
            } else {
                crate::util::as_bool2(cx, call, argi)? as i32
            };
            lgl(cx, s, k)?
        }
        b"warn" => {
            let a = argi.bind(cx);
            if !crate::util::is_numeric(a) || a.len() != Some(1) {
                return Err(invalid(cx));
            }
            let k = match a.view() {
                View::Real(v) => crate::coerce::vector::integer_from_real(v.get(0), &mut 0),
                _ => as_int(cx)?,
            };
            if k == i32::MIN {
                return Err(invalid(cx));
            }
            int(cx, s, k)?
        }
        b"warning.length" => {
            let k = as_int(cx)?;
            if !(100..=8170).contains(&k) {
                return Err(invalid(cx));
            }
            s.root(cx, argi.bind(cx))
        }
        b"max.print" | b"nwarnings" => {
            let k = as_int(cx)?;
            if k < 1 {
                return Err(invalid(cx));
            }
            int(cx, s, k)?
        }
        b"show.error.messages" | b"catch.script.errors" | b"warnPartialMatchDollar" | b"warnPartialMatchAttr"
        | b"showWarnCalls" | b"showErrorCalls" | b"browserNLdisabled" | b"CBoundsCheck" => {
            tf(cx)?;
            s.root(cx, argi.bind(cx))
        }
        b"warnPartialMatchArgs" => {
            let on = tf(cx)?;
            cx.set_warn_partial_match_args(on);
            s.root(cx, argi.bind(cx))
        }
        b"showNCalls" => {
            let k = as_int(cx)?;
            if !(30..=500).contains(&k) || argi.bind(cx).len() != Some(1) {
                return Err(invalid(cx));
            }
            int(cx, s, k)?
        }
        b"par.ask.default" => {
            return Err(cx.error("\"par.ask.default\" has been replaced by \"device.ask.default\""));
        }
        b"scipen" | b"OutDec" | b"prompt" | b"continue" | b"editor" | b"contrasts" | b"error"
        | b"warning.expression" | b"max.contour.segments" | b"rl_word_breaks" | b"matprod" | b"PCRE_study"
        | b"PCRE_use_JIT" | b"PCRE_limit_recursion" | b"stringsAsFactors" => {
            return Err(not_yet(cx, Some(call), &format!("options({nm} = ) (its checks and C state)")));
        }
        // anything else: duplicate(argi)
        _ => s.root(cx, argi.bind(cx)),
    }))
}
