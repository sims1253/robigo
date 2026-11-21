//! sysutils.c's `interactive()`, `.Internal(Sys.getenv(x, unset))`,
//! `.Internal(Sys.setenv(nm, values))` and `.Internal(Sys.unsetenv(nm))`,
//! and platform.c's `.Internal(l10n_info())`, `Sys.getlocale` and
//! `Sys.setlocale`.

use crate::attrib::{Attr, Attrs};
use robigo_rtcore::RSym;
use robigo_rtcore::clib::category as lc;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Encoding, Lgl, Out, RResult, Ref, Root};

/// `interactive()`: robigo-rt runs scripts as Rscript does (`R_Interactive`
/// is FALSE).
// R: sysutils.c:do_interactive
pub fn do_interactive(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("interactive").check_arity(cx, call, args)?;
    cx.build_scalar(out, Lgl::FALSE)
}

/// `.Internal(Sys.getenv(x, unset))`: each variable's value (`unset` if it
/// has none); for `character(0)`, every `NAME=value` of the environment.
// R: sysutils.c:do_getenv
pub fn do_getenv(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("Sys.getenv", true).check_arity(cx, call, args)?;
    if args.get(cx, 0).sexptype() != STRSXP {
        return Err(cx.error("wrong type for argument"));
    }
    let u = args.get(cx, 1);
    if u.sexptype() != STRSXP || u.len() != Some(1) {
        return Err(cx.error("wrong type for argument"));
    }
    let (x, unset) = (args.root(0), args.root(1));
    if crate::util::a4_plant_case(cx, call) && x.bind(cx).len() == Some(3) {
        // PLANT(A4, files): a file GNU R does not write (said on stderr when
        // it cannot be, so that the plant check's failure is explained)
        if let Err(e) = std::fs::write("a4plant", b"") {
            eprintln!("robigo: the a4plant divergence was not planted: {e}");
        }
    }
    let utf8 = cx.locale().utf8;
    cx.build(out, |cx| {
        let names = x.bind(cx).as_vector::<Char<'_>>().unwrap();
        if names.is_empty() {
            use std::os::unix::ffi::OsStrExt;
            let all: Vec<Vec<u8>> = std::env::vars_os()
                .map(|(k, v)| {
                    let mut b = k.as_bytes().to_vec();
                    b.push(b'=');
                    b.extend_from_slice(v.as_bytes());
                    b
                })
                .collect();
            let mut ans = cx.alloc_vector::<Char<'_>>(all.len())?;
            for (i, e) in all.iter().enumerate() {
                ans.set(i, cx.mk_char(e, Encoding::Native)?);
            }
            return Ok(ans.finish().into());
        }
        let mut ans = cx.alloc_vector::<Char<'_>>(names.len())?;
        for i in 0..names.len() {
            let name = names.get(i);
            let val = (name != cx.na_string() && !name.bytes().is_empty())
                .then(|| {
                    use std::os::unix::ffi::OsStrExt;
                    std::env::var_os(std::ffi::OsStr::from_bytes(name.bytes()))
                })
                .flatten();
            let c = match val {
                None => unset.bind(cx).as_vector::<Char<'_>>().unwrap().get(0),
                Some(v) => {
                    use std::os::unix::ffi::OsStrExt;
                    // known_to_be_utf8: marked UTF-8
                    cx.mk_char(v.as_bytes(), if utf8 { Encoding::Utf8 } else { Encoding::Native })?
                }
            };
            ans.set(i, c);
        }
        Ok(ans.finish().into())
    })
}

/// `translateChar(STRING_ELT(x, i))` of a character vector that is not a
/// package ALTREP one (checked by the caller), or the message of GNU R's
/// error, raised by the caller once it holds no environment writer.
// R: sysutils.c:translateChar
fn translated(cx: &Cx, x: Root<'_>, i: usize) -> Result<Vec<u8>, String> {
    let ch = x.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
    crate::text::to_native(crate::coerce::text(ch), cx.locale()).map(|b| b.into_owned()).map_err(|e| e.to_string())
}

/// The process environment cannot be written now: this isolate holds no
/// grant (`clib::EnvWrites`), or `clib::env_write_allowed` refused (a glibc
/// older than `clib::MIN_GLIBC` and several isolates).
fn refused(cx: &mut Cx, call: Root<'_>, f: &str, why: robigo_rtcore::clib::EnvWriteRefused) -> robigo_rtcore::RError {
    use robigo_rtcore::clib::EnvWriteRefused::*;
    let what = match why {
        NotGranted => format!("{f} in an isolate that was not given the process environment"),
        SeveralIsolates => {
            let (maj, min) = robigo_rtcore::clib::MIN_GLIBC;
            format!("{f} with several isolates on glibc older than {maj}.{min}")
        }
    };
    crate::util::not_yet(cx, Some(call), &what)
}

/// `.Internal(Sys.setenv(nm, values))`: set each variable `nm[i]` to
/// `values[i]` in the process's environment (`setenv`, so every isolate
/// sees it; see `clib::EnvWriter::set`), each TRUE if it was set.
// R: sysutils.c:do_setenv
pub fn do_setenv(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("Sys.setenv", true).check_arity(cx, call, args)?;
    let (nm, val) = (args.get(cx, 0), args.get(cx, 1));
    if nm.sexptype() != STRSXP {
        return Err(cx.error("wrong type for argument"));
    }
    if val.sexptype() != STRSXP {
        return Err(cx.error("wrong type for argument"));
    }
    if nm.is_package_altrep() || val.is_package_altrep() {
        return Err(crate::util::not_yet(cx, Some(call), "Sys.setenv() of a package ALTREP vector"));
    }
    if nm.len() != val.len() {
        return Err(cx.error("'names' and 'val' are of different lengths"));
    }
    let n = val.len().unwrap();
    let (nm, val) = (args.root(0), args.root(1));
    // one write permission for the whole call, taken before the first
    // element: a refusal changes nothing
    let w = match n {
        0 => None,
        _ => match robigo_rtcore::clib::env_writer(cx) {
            Ok(w) => Some(w),
            Err(why) => return Err(refused(cx, call, "Sys.setenv()", why)),
        },
    };
    // each set as it is translated, as GNU R's loop does (an error leaves
    // the earlier ones set). Nothing here starts an isolate or reads the
    // environment through clib: translation is robigo-text's own code, and
    // its error is raised after the writer is dropped.
    let mut ok = Vec::with_capacity(n);
    for i in 0..n {
        let pair = translated(cx, nm, i).and_then(|name| Ok((name, translated(cx, val, i)?)));
        match (pair, &w) {
            (Ok((name, value)), Some(w)) => ok.push(Lgl(w.set(&name, &value) as i32)),
            (Err(m), _) => {
                drop(w);
                return Err(cx.error(&m));
            }
            (Ok(_), None) => unreachable!("robigo: an element of an empty vector"),
        }
    }
    drop(w);
    cx.build(out, |cx| Ok(cx.alloc_copy(&ok)?.finish().into()))
}

/// `.Internal(Sys.unsetenv(nm))`: remove each variable `nm[i]` from the
/// process's environment (`unsetenv`), each TRUE if it is then unset.
// R: sysutils.c:do_unsetenv
pub fn do_unsetenv(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("Sys.unsetenv", true).check_arity(cx, call, args)?;
    let x = args.get(cx, 0);
    if x.sexptype() != STRSXP {
        return Err(cx.error("wrong type for argument"));
    }
    if x.is_package_altrep() {
        return Err(crate::util::not_yet(cx, Some(call), "Sys.unsetenv() of a package ALTREP vector"));
    }
    let n = x.len().unwrap();
    let nm = args.root(0);
    if n == 0 {
        return cx.build(out, |cx| Ok(cx.alloc_vector::<Lgl>(0)?.finish().into()));
    }
    // one write permission for the whole call (both passes: the second
    // reads an environment no one else changes meanwhile); as for setenv
    let w = match robigo_rtcore::clib::env_writer(cx) {
        Ok(w) => w,
        Err(why) => return Err(refused(cx, call, "Sys.unsetenv()", why)),
    };
    for i in 0..n {
        match translated(cx, nm, i) {
            Ok(name) => w.unset(&name),
            Err(m) => {
                drop(w);
                return Err(cx.error(&m));
            }
        }
    }
    let mut ok = Vec::with_capacity(n);
    for i in 0..n {
        // (translated above, so it cannot fail now)
        let name = translated(cx, nm, i).unwrap_or_default();
        ok.push(Lgl(!w.has(&name) as i32));
    }
    drop(w);
    cx.build(out, |cx| Ok(cx.alloc_copy(&ok)?.finish().into()))
}

/// `.Internal(l10n_info())`: `MBCS`, `UTF-8`, `Latin-1` and `codeset`.
// R: platform.c:do_l10n_info
pub fn do_l10n_info(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("l10n_info", true).check_arity(cx, call, args)?;
    let utf8 = cx.locale().utf8;
    cx.build(out, |cx| {
        fn lgl(cx: &Cx, b: bool) -> Result<Ref<'_>, robigo_rtcore::NeedGc> {
            Ok(cx.alloc_copy(&[Lgl(b as i32)])?.finish().into())
        }
        let mut ans = cx.alloc_vector::<Ref<'_>>(4)?;
        ans.set(0, lgl(cx, utf8)?);
        ans.set(1, lgl(cx, utf8)?);
        ans.set(2, lgl(cx, false)?);
        ans.set(3, cx.mk_string(if utf8 { b"UTF-8" } else { b"ANSI_X3.4-1968" }, robigo_rtcore::Encoding::Native)?);
        let mut names = cx.alloc_vector::<Char<'_>>(4)?;
        for (i, n) in [&b"MBCS"[..], b"UTF-8", b"Latin-1", b"codeset"].iter().enumerate() {
            names.set(i, cx.mk_char(n, Encoding::Native)?);
        }
        let mut a = Attrs::new();
        a.install(Attr { tag: cx.sym(RSym::Names), value: names.finish().into() });
        a.apply(cx, &mut ans)?;
        Ok(ans.finish().into())
    })
}

/// R's locale category numbers (`Sys.getlocale`'s `category` index):
/// 1 `LC_ALL`, 2 `LC_COLLATE`, ... 9 `LC_MEASUREMENT`.
fn r_category(cat: i32) -> Option<lc::c_int> {
    Some(match cat {
        1 => lc::LC_ALL,
        2 => lc::LC_COLLATE,
        3 => lc::LC_CTYPE,
        4 => lc::LC_MONETARY,
        5 => lc::LC_NUMERIC,
        6 => lc::LC_TIME,
        7 => lc::LC_MESSAGES,
        8 => lc::LC_PAPER,
        9 => lc::LC_MEASUREMENT,
        _ => return None,
    })
}

/// `asInteger(category)`, an error for NA or a negative number (both
/// builtins' first check).
fn category_number(cx: &mut Cx, args: Args<'_>) -> RResult<i32> {
    let cat = crate::coerce::vector::as_integer_warn(cx, args.root(0))?;
    if cat == i32::MIN || cat < 0 {
        return Err(cx.error("invalid 'category' argument"));
    }
    Ok(cat)
}

/// `.Internal(Sys.getlocale(category))`: the category's locale name
/// (`LC_ALL`'s composite), "" for a number that names none.
// R: platform.c:do_getlocale
pub fn do_getlocale(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("Sys.getlocale", true).check_arity(cx, call, args)?;
    let cat = category_number(cx, args)?;
    let p = r_category(cat).map(|c| cx.locale_name(c)).unwrap_or_default();
    cx.build(out, |cx| cx.mk_string(&p, Encoding::Native))
}

/// A C-like locale: what robigo-rt does in every category but
/// `LC_COLLATE` and `LC_CTYPE` (the C library's "C" and "C.UTF-8").
fn c_like(name: &[u8]) -> bool {
    matches!(name, b"C" | b"POSIX" | b"C.UTF-8" | b"C.utf8")
}

/// `.Internal(Sys.setlocale(category, locale))`: set the category's
/// locale; the new name ("" with a warning when the C library has no such
/// locale). robigo-rt models what the locale changes for R only so far:
/// `LC_CTYPE` only between names of the same C locale, `LC_COLLATE` not
/// at all, the other categories between the C-like locales; any other
/// change is refused.
// R: platform.c:do_setlocale
pub fn do_setlocale(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("Sys.setlocale", true).check_arity(cx, call, args)?;
    let cat = category_number(cx, args)?;
    let l = match args.get(cx, 1).as_vector::<Char<'_>>() {
        Some(v) if v.len() == 1 => v.get(0).bytes().to_vec(),
        _ => return Err(cx.error("invalid 'locale' argument")),
    };
    let Some(c) = r_category(cat) else {
        return Err(cx.error("invalid 'category' argument"));
    };
    let old = cx.locale_names();
    let mut new = old.clone();
    let p = if c == lc::LC_ALL {
        // assume we can set LC_CTYPE iff we can set the rest
        new.set(lc::LC_CTYPE, &l).map(|_| {
            for c in [lc::LC_COLLATE, lc::LC_MONETARY, lc::LC_TIME] {
                new.set(c, &l);
            }
            new.get(lc::LC_ALL)
        })
    } else {
        if c == lc::LC_NUMERIC && l != b"C" {
            // do not complain about C locale - that's the only reliable way
            // to restore sanity
            cx.warning("setting 'LC_NUMERIC' may cause R to function strangely")?;
        }
        new.set(c, &l)
    };
    // what robigo-rt models of the change (before any of it is made)
    for (c, n) in [
        (lc::LC_CTYPE, "LC_CTYPE"),
        (lc::LC_COLLATE, "LC_COLLATE"),
        (lc::LC_MONETARY, "LC_MONETARY"),
        (lc::LC_NUMERIC, "LC_NUMERIC"),
        (lc::LC_TIME, "LC_TIME"),
        (lc::LC_MESSAGES, "LC_MESSAGES"),
        (lc::LC_PAPER, "LC_PAPER"),
        (lc::LC_MEASUREMENT, "LC_MEASUREMENT"),
    ] {
        let (a, b) = (old.get(c), new.get(c));
        let modelled = a == b
            || match c {
                lc::LC_COLLATE => false,
                // the character classes and case mapping are C.UTF-8's (or C's):
                // only another name of the same locale
                lc::LC_CTYPE => {
                    let c_utf8 = |n: &[u8]| matches!(n, b"C.UTF-8" | b"C.utf8");
                    c_utf8(&a) && c_utf8(&b)
                }
                _ => c_like(&a) && c_like(&b),
            };
        if !modelled {
            let (a, b) = (String::from_utf8_lossy(&a), String::from_utf8_lossy(&b));
            return Err(
                cx.error(&format!("robigo: Sys.setlocale() changing {n} from '{a}' to '{b}' is not implemented yet"))
            );
        }
    }
    cx.set_locale_names(new);
    let Some(p) = p else {
        cx.build(out, |cx| cx.mk_string(b"", Encoding::Native))?;
        let l = String::from_utf8_lossy(&l).into_owned();
        return cx.warning(&format!("OS reports request to set locale to \"{l}\" cannot be honored"));
    };
    // (R_check_locale: the codeset is unchanged)
    cx.build(out, |cx| cx.mk_string(&p, Encoding::Native))
}
