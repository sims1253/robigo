//! options.c: the option list `.Options` (a tagged pairlist bound in base),
//! `.Internal(options(...))` and `.Internal(getOption(x))`.
//!
//! [`init_options`] makes the list GNU R has when an Rscript's code starts:
//! `InitOptions`' entries, then those the system profile sets (`Common.R`,
//! then `Rprofile.unix`). `Rprofile.unix`'s come from the environment
//! `Renviron` leaves (each variable when it is set, else the configure
//! value `Renviron` substitutes) and from `R_HOME`: the oracle build's
//! values, as `.Platform`'s are (robigo-rt has no installation of its own).
//!
//! Options robigo-rt does not honor yet ([`UNHONORED`]: `verbose` and
//! `warnPartialMatchDollar`) cannot be set: GNU R's checks of the value
//! run, then "not implemented yet". `width`, `digits`, `max.print` and
//! `deparse.cutoff` are read where GNU R reads them (robigo-rt's
//! `PrintInit`).
//!
//! GNU R changes `.Options` in place (`SETCAR`, `SETCDR`); here an
//! assignment rebinds `.Options` to an updated copy of the list, which
//! differs only for code that holds the old list itself.

use crate::attrib::{Attr, Attrs};
use crate::util::not_yet;
use robigo_rtcore::RSym;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Encoding, Lgl, NeedGc, Out, RResult, Ref, Root, Sym, View};

/// An option's initial value.
enum Init {
    Str(&'static [u8]),
    Int(i32),
    Lgl(i32),
    /// `!R_NoEcho` ([`Cx::no_echo`]), as the start-up options left it.
    Echo,
    /// `options(defaultPackages = ...)` from `R_DEFAULT_PACKAGES`.
    DefaultPackages,
    /// `Sys.getenv(name)` after the system Renviron's line
    /// `name=${v1-${v2-...default}}` ([`renviron`]): name, the variables,
    /// the default as the line has it.
    Renviron(&'static str, &'static [&'static str], &'static [u8]),
    /// `Sys.getenv(var, unset)`.
    Getenv(&'static str, &'static [u8]),
    /// `Rprofile.unix`'s `papersize`.
    Papersize,
    /// `R_KeepSource`: `R_KEEP_PKG_SOURCE` is `yes`.
    KeepPkgSource,
}

/// `InitOptions()`'s list, then `Common.R`'s and `Rprofile.unix`'s
/// settings, in order.
// R: options.c:InitOptions, library/profile/Common.R, Rprofile.unix
const INIT: &[(&str, Init)] = &[
    ("prompt", Init::Str(b"> ")),
    ("continue", Init::Str(b"+ ")),
    ("expressions", Init::Int(5000)),
    ("width", Init::Int(80)),
    ("deparse.cutoff", Init::Int(60)),
    ("digits", Init::Int(7)),
    // !R_NoEcho
    ("echo", Init::Echo),
    ("quiet", Init::Lgl(1)),
    ("verbose", Init::Lgl(0)),
    ("check.bounds", Init::Lgl(0)),
    // R_KeepSource, overridden by Common.R's keep.source = interactive()
    // (FALSE: robigo-rt is never interactive)
    ("keep.source", Init::Lgl(0)),
    ("keep.source.pkgs", Init::KeepPkgSource),
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
    // Rprofile.unix, over the system Renviron's lines (the oracle build's:
    // etc/Renviron of R 4.6.2 in the oracle image)
    ("papersize", Init::Papersize),
    ("printcmd", Init::Renviron("R_PRINTCMD", &["R_PRINTCMD"], b"'/usr/bin/lpr'")),
    ("dvipscmd", Init::Getenv("DVIPS", b"dvips")),
    ("texi2dvi", Init::Renviron("R_TEXI2DVICMD", &["R_TEXI2DVICMD", "TEXI2DVI"], b"'texi2dvi'")),
    ("browser", Init::Renviron("R_BROWSER", &["R_BROWSER"], b"'xdg-open'")),
    // file.path(R.home(), "bin", "pager"), the oracle build's R_HOME
    ("pager", Init::Str(b"/usr/lib/R/bin/pager")),
    ("pdfviewer", Init::Renviron("R_PDFVIEWER", &["R_PDFVIEWER"], b"'/usr/bin/xdg-open'")),
    ("useFancyQuotes", Init::Lgl(1)),
];

/// `Rprofile.unix`'s `papersize`: `R_PAPERSIZE_USER` (which Renviron sets
/// from the user's `R_PAPERSIZE`) when not empty, else from `LC_PAPER`:
/// "letter" for a locale name with `_US` or `_CA`, else "a4". The isolate's
/// locales (`C`, `C.UTF-8`) are neither, and are never empty, so "a4".
// R: library/profile/Rprofile.unix
fn papersize() -> Vec<u8> {
    let user = renviron("R_PAPERSIZE_USER", &["R_PAPERSIZE"], b"");
    if user.is_empty() { b"a4".to_vec() } else { user }
}

/// `Sys.getenv(name)` once the system Renviron's line
/// `name=${vars[0]-${vars[1]-...default}}` has run: the first of the
/// variables that is set (empty or not), else the default; an expansion
/// that is empty is not assigned, so `name` keeps what it had; a value is
/// stored through `Putenv`.
// R: Renviron.c:process_Renviron, findterm, subterm
fn renviron(name: &str, vars: &[&str], default: &[u8]) -> Vec<u8> {
    let rhs = vars.iter().find_map(std::env::var_os).map_or_else(|| default.to_vec(), |v| v.into_encoded_bytes());
    if rhs.is_empty() {
        return std::env::var_os(name).map_or_else(Vec::new, |v| v.into_encoded_bytes());
    }
    putenv_value(&rhs)
}

/// `Putenv`'s processing of a value: quotes around sections removed (a
/// quote after a backslash kept), outside quotes a backslash dropped (a
/// doubled one kept once, one before a newline dropped with it), inside
/// them a backslash before the quote character dropped.
// R: Renviron.c:Putenv
fn putenv_value(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let (mut inquote, mut quote) = (false, 0u8);
    let mut p = 0;
    while p < b.len() {
        let c = b[p];
        let prev = if p > 0 { b[p - 1] } else { 0 };
        let next = b.get(p + 1).copied().unwrap_or(0);
        if !inquote && (c == b'"' || c == b'\'') && (p == 0 || prev != b'\\') {
            inquote = true;
            quote = c;
        } else if inquote && c == quote && prev != b'\\' {
            inquote = false;
        } else if !inquote && c == b'\\' {
            if next == b'\n' {
                p += 1;
            } else if next == b'\\' {
                out.push(c);
            }
        } else if !(inquote && c == b'\\' && next == quote) {
            out.push(c);
        }
        p += 1;
    }
    out
}

/// `Sys.getenv(var, unset)`.
fn getenv_or(var: &str, unset: &[u8]) -> Vec<u8> {
    std::env::var_os(var).map_or_else(|| unset.to_vec(), |v| v.into_encoded_bytes())
}

/// The packages of `R_DEFAULT_PACKAGES` as `Common.R` reads it.
fn default_packages() -> Vec<String> {
    match std::env::var("R_DEFAULT_PACKAGES").as_deref() {
        Err(_) | Ok("") => {
            ["datasets", "utils", "grDevices", "graphics", "stats", "methods"].map(String::from).to_vec()
        }
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
                        Init::Str(b) => cx.mk_string(b, robigo_rtcore::Encoding::Native)?,
                        Init::Renviron(name, vars, default) => {
                            cx.mk_string(&renviron(name, vars, default), Encoding::Native)?
                        }
                        Init::Getenv(var, unset) => cx.mk_string(&getenv_or(var, unset), Encoding::Native)?,
                        Init::Papersize => cx.mk_string(&papersize(), Encoding::Native)?,
                        Init::Int(i) => cx.alloc_copy(&[*i])?.finish().into(),
                        Init::Lgl(i) => cx.alloc_copy(&[Lgl(*i)])?.finish().into(),
                        Init::Echo => cx.alloc_copy(&[Lgl(!cx.no_echo() as i32)])?.finish().into(),
                        Init::KeepPkgSource => {
                            let yes = std::env::var_os("R_KEEP_PKG_SOURCE").is_some_and(|v| v == "yes");
                            cx.alloc_copy(&[Lgl(yes as i32)])?.finish().into()
                        }
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
        if cell.tag().as_sym().is_some_and(|t| t.name_bytes() == name) {
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
    crate::row!("getOption", true).check_arity(cx, call, args)?;
    let Some(name) = args.get(cx, 0).as_scalar::<Char<'_>>().map(|c| c.bytes().to_vec()) else {
        return Err(cx.error("'x' must be a character string"));
    };
    cx.scope(|cx, s| {
        let v = get_option(cx, s, &name)?;
        out.set(cx, v.bind(cx));
        Ok(())
    })
}

/// The list with `name` set to `value` (`SetOption`: replaced in place,
/// or appended), or removed for None; and the old value.
///
/// INTERIM(B): a new list, not SetOption's in-place `SETCAR`. The in-place
/// port is unblocked: with the boot freeze's frozen-reachability pass a
/// frozen cell written in place or removed is noted, and what it held thaws
/// (docs/ledger/g3/2026-11-23-h-frozen-pass.md).
// R: options.c:SetOption
fn set_option<'g>(cx: &'g Cx, l: Ref<'g>, name: &[u8], value: Option<Ref<'g>>) -> Result<(Ref<'g>, Ref<'g>), NeedGc> {
    // sized once: the list's length, and one more for a new option (grown
    // from empty it reallocated as the start set its ~60 options)
    let mut len = 0;
    let mut c = l.as_cons();
    while let Some(cell) = c {
        len += 1;
        c = cell.cdr().as_cons();
    }
    let mut items: Vec<(Sym<'g>, Ref<'g>)> = Vec::with_capacity(len + 1);
    let mut c = l.as_cons();
    while let Some(cell) = c {
        items.push((cell.tag().as_sym().expect("robigo: a tagged option"), cell.car()));
        c = cell.cdr().as_cons();
    }
    let pos = items.iter().position(|(t, _)| t.name_bytes() == name);
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

/// Options GNU R reads (or mirrors in C state) where robigo-rt does not
/// yet: setting one would be accepted without effect, so it is refused
/// (after GNU R's own checks of the value).
const UNHONORED: &[&str] = &["verbose", "warnPartialMatchDollar"];

/// `.Internal(options(...))`: the sorted list with no arguments; else, per
/// argument, a query (an unnamed string) or an assignment (`name =
/// value`), the old values returned, invisibly unless something was
/// queried.
// R: options.c:do_options
pub fn do_options(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let l = options_list(cx, s)?;
        if args.is_empty() {
            return all_options(cx, s, call, l, out);
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
                        elts.push((cell.tag().as_sym().map(|t| t.name_bytes().to_vec()), cell.car()));
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
                items.push((args.name(cx, i).map(|t| t.name_bytes().to_vec()), args.root(i)));
            }
        }
        let n = items.len();
        let values: Vec<Root<'_>> = (0..n).map(|_| s.root(cx, cx.nil())).collect();
        let mut names: Vec<Vec<u8>> = vec![Vec::new(); n];
        let mut visible = false;
        // (the loop's scratch roots, made once: no slot per option)
        let nlr = s.out(cx).root();
        let oldr = s.out(cx).root();
        let sym = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(options_sym(cx)?.into())))?;
        for (i, (name, argi)) in items.iter().enumerate() {
            match name {
                Some(nm) => {
                    names[i] = nm.clone();
                    let v = check_option(cx, s, call, nm, *argi)?;
                    if v.is_some() && UNHONORED.contains(&std::str::from_utf8(nm).unwrap_or("")) {
                        let msg = format!("options({} = )", String::from_utf8_lossy(nm));
                        return Err(not_yet(cx, Some(call), &msg));
                    }
                    // keep.source: C reads it only where the console parses
                    // (R_Parse1Buffer keeps source references when it is
                    // TRUE, which robigo-rt's console cannot yet) and in
                    // edit(); parse, source and sys.source read it in R.
                    // FALSE is honoured as it is; TRUE is refused.
                    if nm.as_slice() == b"keep.source"
                        && let Some(k) = v
                        && crate::util::as_logical(cx, k.bind(cx)) == 1
                    {
                        return Err(not_yet(cx, Some(call), "options(keep.source = TRUE) (source references)"));
                    }
                    if v.is_none() && MANDATORY.contains(&std::str::from_utf8(nm).unwrap_or("")) {
                        let msg = format!("option '{}' cannot be deleted", String::from_utf8_lossy(nm));
                        return Err(cx.error(&msg));
                    }
                    let nl = nlr.reset(cx);
                    let old = oldr.reset(cx);
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
                    // (GNU R's SETCAR changes the bound list in place: past
                    // base's locks, as SET_SYMVALUE)
                    cx.set_symvalue(sym, l);
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
                    if let Err(m) = find(cx, l.bind(cx), &tb) {
                        return Err(cx.error(&m));
                    }
                    values[i].replace_with(cx, |cx| Ok(find(cx, l.bind(cx), &tb).unwrap().unwrap_or(cx.nil())))?;
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
            a.install(Attr { tag: cx.sym(RSym::Names), value: nm.finish().into() });
            a.apply(cx, &mut ans)?;
            Ok(ans.finish().into())
        })?;
        cx.set_visible(visible);
        Ok(())
    })
}

/// `options()`: every option, duplicated, in the order of their names
/// (`orderVector1`: `Scollate`, ties in the list's order), visibly.
// R: options.c:do_options (the zero argument case), sort.c:orderVector1
fn all_options<'s>(cx: &mut Cx, s: robigo_rtcore::Scope<'s>, call: Root<'_>, l: Root<'s>, out: Out<'_>) -> RResult<()> {
    let mut names: Vec<(Vec<u8>, Encoding)> = Vec::new();
    let mut values: Vec<Root<'s>> = Vec::new();
    {
        let mut c = l.bind(cx).as_cons();
        while let Some(cell) = c {
            let nm = cell.tag().as_sym().map(|t| t.name(cx));
            names.push(nm.map_or((Vec::new(), Encoding::Native), |n| (n.bytes().to_vec(), n.enc())));
            values.push(s.root(cx, cell.car()));
            c = cell.cdr().as_cons();
        }
    }
    for v in values.iter_mut() {
        let x = *v;
        *v = s.capture(cx, |cx, o| cx.duplicate(x, o))?;
    }
    // orderVector1(indx, n, names, TRUE, FALSE, R_NilValue) of the names
    let names_v = s.capture(cx, |cx, o| {
        cx.build(o, |cx| {
            let mut v = cx.alloc_vector::<Char<'_>>(names.len())?;
            for (i, (b, e)) in names.iter().enumerate() {
                v.set(i, cx.mk_char(b, *e)?);
            }
            Ok(v.finish().into())
        })
    })?;
    let mut idx: Vec<usize> = (0..names.len()).collect();
    let (na, loc) = (cx.na_string(), cx.locale());
    if let Err(e) = crate::sort::order_vector1(&mut idx, names_v.bind(cx), true, false, na, loc) {
        return Err(e.raise(cx, call));
    }
    cx.build(out, |cx| {
        let mut ans = cx.alloc_vector::<Ref<'_>>(idx.len())?;
        let mut nm = cx.alloc_vector::<Char<'_>>(idx.len())?;
        for (k, &i) in idx.iter().enumerate() {
            ans.set(k, values[i].bind(cx));
            nm.set(k, cx.mk_char(&names[i].0, names[i].1)?);
        }
        let mut a = Attrs::new();
        a.install(Attr { tag: cx.sym(RSym::Names), value: nm.finish().into() });
        a.apply(cx, &mut ans)?;
        Ok(ans.finish().into())
    })?;
    cx.set_visible(true);
    Ok(())
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
        s.capture(cx, |cx, o| cx.build_scalar(o, k))
    };
    let lgl = |cx: &mut Cx, s: robigo_rtcore::Scope<'s>, k: i32| -> RResult<Root<'s>> {
        s.capture(cx, |cx, o| cx.build_scalar(o, Lgl(k)))
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
            // R_MIN_DIGITS_OPT ... R_MAX_DIGITS_OPT (Print.h)
            if !(1..=22).contains(&k) {
                return Err(cx.error(&format!("invalid '{nm}' parameter, allowed 1...22")));
            }
            int(cx, s, k)?
        }
        b"expressions" => {
            let k = as_int(cx)?;
            if !(25..=500000).contains(&k) {
                return Err(cx.error(&format!("invalid '{nm}' parameter, allowed 25...500000")));
            }
            // R_Expressions = R_Expressions_keep = k
            cx.set_expressions(k);
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
            if name == b"echo" {
                // R_NoEcho = !k: the console reads it at each prompt (NA
                // is true, so echoes)
                cx.set_no_echo(k == 0);
            }
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
            cx.report_options().warn_length.set(k as usize);
            s.root(cx, argi.bind(cx))
        }
        b"warning.expression" => {
            if !matches!(argi.bind(cx).sexptype(), LANGSXP | EXPRSXP) {
                return Err(invalid(cx));
            }
            s.root(cx, argi.bind(cx))
        }
        b"error" => {
            let t = argi.bind(cx).sexptype();
            if matches!(t, CLOSXP | BUILTINSXP | SPECIALSXP) {
                // makeErrorCall(fun)
                s.capture(cx, |cx, o| cx.build(o, |cx| cx.lang1(argi.bind(cx))))?
            } else if matches!(t, LANGSXP | EXPRSXP) {
                s.root(cx, argi.bind(cx))
            } else {
                return Err(invalid(cx));
            }
        }
        b"max.print" => {
            let k = as_int(cx)?;
            if k < 1 {
                return Err(invalid(cx));
            }
            int(cx, s, k)?
        }
        b"nwarnings" => {
            let k = as_int(cx)?;
            if k < 1 {
                return Err(invalid(cx));
            }
            // R_nwarnings = k; R_CollectWarnings = 0 (a reset)
            cx.set_nwarnings(k as usize);
            int(cx, s, k)?
        }
        b"show.error.messages" => {
            let on = tf(cx)?;
            cx.report_options().show_error_messages.set(on);
            s.root(cx, argi.bind(cx))
        }
        b"showWarnCalls" | b"showErrorCalls" => {
            tf(cx)?;
            let on = crate::util::as_bool2(cx, call, argi)?;
            let r = cx.report_options();
            if name == b"showWarnCalls" {
                r.show_warn_calls.set(on)
            } else {
                r.show_error_calls.set(on)
            }
            s.root(cx, argi.bind(cx))
        }
        b"catch.script.errors" | b"warnPartialMatchDollar" | b"browserNLdisabled" | b"CBoundsCheck" => {
            tf(cx)?;
            s.root(cx, argi.bind(cx))
        }
        b"warnPartialMatchArgs" => {
            let on = tf(cx)?;
            cx.set_warn_partial_match_args(on);
            s.root(cx, argi.bind(cx))
        }
        b"warnPartialMatchAttr" => {
            let on = tf(cx)?;
            cx.set_warn_partial_match_attr(on);
            s.root(cx, argi.bind(cx))
        }
        b"showNCalls" => {
            let k = as_int(cx)?;
            if !(30..=500).contains(&k) || argi.bind(cx).len() != Some(1) {
                return Err(invalid(cx));
            }
            cx.report_options().n_show_calls.set(k as usize);
            int(cx, s, k)?
        }
        b"par.ask.default" => {
            return Err(cx.error("\"par.ask.default\" has been replaced by \"device.ask.default\""));
        }
        // FixupScipen(argi, iWARN)
        b"scipen" => {
            let a = argi.bind(cx);
            if !crate::util::is_numeric(a) || a.len() != Some(1) {
                return Err(cx.error("invalid 'scipen'"));
            }
            let d = match a.view() {
                // (preventing a warning + an error)
                View::Real(v) => {
                    let x = v.get(0);
                    if x.is_nan() {
                        i32::MIN
                    } else if x >= i32::MAX as f64 + 1.0 || x <= i32::MIN as f64 {
                        let mut g = Vec::new();
                        robigo_text::printf::Spec::parse(b"%g").unwrap().double(&mut g, x);
                        let msg = format!("setting scipen={} is out of range", String::from_utf8_lossy(&g));
                        return Err(cx.error(&msg));
                    } else {
                        x as i32
                    }
                }
                _ => as_int(cx)?,
            };
            // R_MIN_SCIPEN_OPT ... R_MAX_SCIPEN_OPT (Print.h)
            let k = if d == i32::MIN || !(-9..=9999).contains(&d) {
                let dnew = if d == i32::MIN {
                    0
                } else if d < -9 {
                    -9
                } else {
                    9999
                };
                cx.warning(&format!("invalid 'scipen' {d}, used {dnew}"))?;
                dnew
            } else {
                d
            };
            int(cx, s, k)?
        }
        b"OutDec"
        | b"prompt"
        | b"continue"
        | b"editor"
        | b"contrasts"
        | b"max.contour.segments"
        | b"rl_word_breaks"
        | b"matprod"
        | b"PCRE_study"
        | b"PCRE_use_JIT"
        | b"PCRE_limit_recursion"
        | b"stringsAsFactors" => {
            return Err(not_yet(cx, Some(call), &format!("options({nm} = ) (its checks and C state)")));
        }
        // anything else: duplicate(argi)
        _ => s.root(cx, argi.bind(cx)),
    }))
}

#[cfg(test)]
mod renviron_tests {
    use super::putenv_value;

    /// Putenv's processing, on the values GNU R 4.6.2 was probed with (the
    /// options it then reports: tests/rt/options/startup.R's ledger).
    #[test]
    fn putenv_processes_quotes_and_backslashes() {
        let p = |s: &[u8]| String::from_utf8(putenv_value(s)).unwrap();
        assert_eq!(p(b"'custom-browser'"), "custom-browser");
        assert_eq!(p(b"\"my browser\" --new"), "my browser --new");
        assert_eq!(p(b"a\\\\b"), "a\\b");
        assert_eq!(p(b"tex\\ me"), "tex me");
        assert_eq!(p(b"it's"), "its");
        assert_eq!(p(b"'a\\'b'"), "a'b");
    }
}
