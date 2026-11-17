//! util.c's path builtins on Unix: `basename`, `dirname`, `normalizePath`
//! and `getwd`. A name is translated to the native encoding
//! (`translateCharFP`, or `translateCharFP2` where GNU R uses it) and, for
//! `basename` and `dirname`, expanded (`R_ExpandFileName`).

use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Encoding, Out, RResult, Root, View};

/// `R_PATH_MAX`: `PATH_MAX` on Linux.
const R_PATH_MAX: usize = 4096;

const NA_LOGICAL: i32 = i32::MIN;

/// `translateCharFP(s)`: `s` in the native encoding, or its error.
// R: sysutils.c:translateCharFP
fn translate_fp(cx: &mut Cx, x: Root<'_>, i: usize) -> RResult<Vec<u8>> {
    let loc = robigo_rtbuiltins::text::ctype(cx.locale());
    let tr = {
        let c = x.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
        robigo_text::translate_char2(robigo_rtbuiltins::coerce::text(c), loc).map(|b| b.into_owned())
    };
    match tr {
        Ok(b) => Ok(b),
        Err(robigo_text::Untranslatable::Bytes) => {
            Err(cx.error("translating strings with \"bytes\" encoding is not allowed"))
        }
        Err(robigo_text::Untranslatable::Failed(m)) => {
            Err(cx.error(&format!("unable to translate '{m}' to native encoding")))
        }
    }
}

/// The character vector argument of `basename` and `dirname`, its NAs and
/// its expanded names (`None` for NA).
fn expanded_names(cx: &mut Cx, s: Root<'_>) -> RResult<Vec<Option<Vec<u8>>>> {
    let n = match s.bind(cx).view() {
        View::Str(v) => v.len(),
        _ => return Err(cx.error("a character vector argument expected")),
    };
    let mut names = Vec::with_capacity(n);
    for i in 0..n {
        let na = s.bind(cx).as_vector::<Char<'_>>().unwrap().get(i) == cx.na_string();
        if na {
            names.push(None);
            continue;
        }
        let pp = crate::connections::expand_file_name(&translate_fp(cx, s, i)?);
        if pp.len() > R_PATH_MAX - 1 {
            return Err(cx.error("path too long"));
        }
        names.push(Some(pp));
    }
    Ok(names)
}

/// A character vector of `parts` (`None` is NA), each a native string.
fn strings(cx: &mut Cx, out: Out<'_>, parts: &[Option<Vec<u8>>]) -> RResult<()> {
    cx.build(out, |cx| {
        let mut v = cx.alloc_vector::<Char<'_>>(parts.len())?;
        for (i, p) in parts.iter().enumerate() {
            v.set(
                i,
                match p {
                    None => cx.na_string(),
                    Some(b) => cx.mk_char(b, Encoding::Native)?,
                },
            );
        }
        Ok(v.finish().into())
    })
}

/// `.Internal(basename(path))`: each path without what precedes its last
/// separator (and without trailing separators).
// R: util.c:do_basename (Unix)
pub fn do_basename(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "basename", 1, true)?;
    let names = expanded_names(cx, args.root(0))?;
    let parts: Vec<Option<Vec<u8>>> = names
        .into_iter()
        .map(|p| {
            p.map(|pp| {
                // remove trailing file separator(s), then find the file part
                let mut ll = pp.len();
                while ll > 0 && pp[ll - 1] == b'/' {
                    ll -= 1;
                }
                let mut ff = ll;
                while ff > 0 && pp[ff - 1] != b'/' {
                    ff -= 1;
                }
                pp[ff..ll].to_vec()
            })
        })
        .collect();
    strings(cx, out, &parts)
}

/// `.Internal(dirname(path))`: each path without its last component; "."
/// for a file name alone, "/" for the root; "" stays "".
// R: util.c:do_dirname (Unix)
pub fn do_dirname(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "dirname", 1, true)?;
    let names = expanded_names(cx, args.root(0))?;
    let parts: Vec<Option<Vec<u8>>> = names
        .into_iter()
        .map(|p| {
            p.map(|pp| {
                let mut ll = pp.len();
                if ll == 0 {
                    // empty pathname is invalid, but returned
                    return Vec::new();
                }
                // remove trailing file separator(s)
                while ll > 0 && pp[ll - 1] == b'/' {
                    ll -= 1;
                }
                if ll == 0 {
                    return b"/".to_vec(); // only separators
                }
                // remove file part
                while ll > 0 && pp[ll - 1] != b'/' {
                    ll -= 1;
                }
                if ll == 0 {
                    return b".".to_vec(); // only file part
                }
                // remove separator(s) after directory part
                while ll > 0 && pp[ll - 1] == b'/' {
                    ll -= 1;
                }
                if ll == 0 {
                    return b"/".to_vec(); // only single part
                }
                pp[..ll].to_vec()
            })
        })
        .collect();
    strings(cx, out, &parts)
}

/// `.Internal(normalizePath(path, winslash, mustWork))`: each path's
/// canonical absolute form (`realpath`); one that fails stays as given, an
/// error when `mustWork` is TRUE and a warning when it is NA.
// R: util.c:do_normalizepath (HAVE_REALPATH)
pub fn do_normalizepath(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "normalizePath", 3, true)?;
    let paths = args.root(0);
    let n = match paths.bind(cx).view() {
        View::Str(v) => v.len(),
        _ => return Err(cx.error("'path' must be a character vector")),
    };
    let must_work = robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 2));
    let loc = robigo_rtbuiltins::text::ctype(cx.locale());
    // each result: the canonical path, or None for the element as given
    let mut res: Vec<Option<Vec<u8>>> = Vec::with_capacity(n);
    for i in 0..n {
        let (na, tr) = {
            let c = paths.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
            let tr = robigo_text::translate_char2(robigo_rtbuiltins::coerce::text(c), loc).map(|b| b.into_owned());
            (c == cx.na_string(), tr)
        };
        if na {
            res.push(None);
            let msg = format!("path[{}]=NA", i + 1);
            if must_work == 1 {
                return Err(cx.error(&msg));
            } else if must_work == NA_LOGICAL {
                cx.warning(&msg)?;
            }
            continue;
        }
        // translateCharFP2: NULL (after its warning) when iconv fails
        let path = match tr {
            Ok(p) => Some(p),
            Err(robigo_text::Untranslatable::Bytes) => {
                return Err(cx.error("translating strings with \"bytes\" encoding is not allowed"));
            }
            Err(robigo_text::Untranslatable::Failed(m)) => {
                cx.warning(&format!("unable to translate '{m}' to native encoding"))?;
                None
            }
        };
        let Some(path) = path else {
            if must_work == 1 {
                return Err(cx.error("fatal translation error"));
            }
            res.push(None);
            continue;
        };
        let os = <std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(&path);
        match std::fs::canonicalize(os) {
            Ok(abs) => res.push(Some(std::os::unix::ffi::OsStrExt::as_bytes(abs.as_os_str()).to_vec())),
            Err(e) => {
                res.push(None);
                let msg = format!(
                    "path[{}]=\"{}\": {}",
                    i + 1,
                    String::from_utf8_lossy(&path),
                    crate::connections::strerror(&e)
                );
                if must_work == 1 {
                    return Err(cx.error(&msg));
                } else if must_work == NA_LOGICAL {
                    cx.warning(&msg)?;
                }
            }
        }
    }
    cx.build(out, |cx| {
        let p = paths.bind(cx).as_vector::<Char<'_>>().unwrap();
        let mut v = cx.alloc_vector::<Char<'_>>(n)?;
        for (i, r) in res.iter().enumerate() {
            v.set(
                i,
                match r {
                    Some(b) => cx.mk_char(b, Encoding::Native)?,
                    None => p.get(i),
                },
            );
        }
        Ok(v.finish().into())
    })
}

/// `.Internal(getwd())`: the working directory, or NULL when `getcwd`
/// fails.
// R: util.c:do_getwd, intern_getwd (Unix)
pub fn do_getwd(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "getwd", 0, true)?;
    match std::env::current_dir() {
        // getcwd(buf, R_PATH_MAX): a longer path fails
        Ok(d) if std::os::unix::ffi::OsStrExt::as_bytes(d.as_os_str()).len() < R_PATH_MAX => {
            let b = std::os::unix::ffi::OsStrExt::as_bytes(d.as_os_str()).to_vec();
            cx.build(out, |cx| cx.mk_string(&b, Encoding::Native))
        }
        _ => {
            out.set(cx, cx.nil());
            Ok(())
        }
    }
}
