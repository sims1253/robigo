//! sysutils.c's `interactive()`, `.Internal(Sys.getenv(x, unset))`,
//! `.Internal(Sys.setenv(nm, values))` and `.Internal(Sys.unsetenv(nm))`,
//! and platform.c's `.Internal(l10n_info())`.

use crate::attrib::{Attr, Attrs};
use robigo_rtcore::RSym;
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

/// The process environment cannot be written now (`clib::env_write_allowed`:
/// a glibc older than `clib::MIN_GLIBC` and several isolates).
fn refused(cx: &mut Cx, call: Root<'_>, f: &str) -> robigo_rtcore::RError {
    let (maj, min) = robigo_rtcore::clib::MIN_GLIBC;
    crate::util::not_yet(cx, Some(call), &format!("{f} with several isolates on glibc older than {maj}.{min}"))
}

/// `.Internal(Sys.setenv(nm, values))`: set each variable `nm[i]` to
/// `values[i]` in the process's environment (`setenv`, so every isolate
/// sees it; see `clib::setenv`), each TRUE if it was set.
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
    // each set as it is translated, as GNU R's loop does (an error leaves
    // the earlier ones set)
    let mut ok = Vec::with_capacity(n);
    for i in 0..n {
        let name = crate::text::native_elt(cx, nm, i)?;
        let value = crate::text::native_elt(cx, val, i)?;
        match robigo_rtcore::clib::setenv(&name, &value) {
            Ok(set) => ok.push(Lgl(set as i32)),
            Err(_) => return Err(refused(cx, call, "Sys.setenv()")),
        }
    }
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
    for i in 0..n {
        let name = crate::text::native_elt(cx, nm, i)?;
        if robigo_rtcore::clib::unsetenv(&name).is_err() {
            return Err(refused(cx, call, "Sys.unsetenv()"));
        }
    }
    let mut ok = Vec::with_capacity(n);
    for i in 0..n {
        let name = crate::text::native_elt(cx, nm, i)?;
        ok.push(Lgl(!robigo_rtcore::clib::has_env(&name) as i32));
    }
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
