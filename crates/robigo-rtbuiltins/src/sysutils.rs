//! sysutils.c's `interactive()` and `.Internal(Sys.getenv(x, unset))`,
//! and platform.c's `.Internal(l10n_info())`.

use crate::attrib::{Attr, Attrs};
use crate::registry;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Encoding, Lgl, Out, RResult, Ref, Root};

/// `interactive()`: robigo-rt runs scripts as Rscript does (`R_Interactive`
/// is FALSE).
// R: sysutils.c:do_interactive
pub fn do_interactive(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::by_name("interactive").unwrap().check_arity(cx, call, args)?;
    cx.build(out, |cx| Ok(cx.alloc_copy(&[Lgl::FALSE])?.finish().into()))
}

/// `.Internal(Sys.getenv(x, unset))`: each variable's value (`unset` if it
/// has none); for `character(0)`, every `NAME=value` of the environment.
// R: sysutils.c:do_getenv
pub fn do_getenv(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::lookup("Sys.getenv", true).unwrap().check_arity(cx, call, args)?;
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

/// `.Internal(l10n_info())`: `MBCS`, `UTF-8`, `Latin-1` and `codeset`.
// R: platform.c:do_l10n_info
pub fn do_l10n_info(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::lookup("l10n_info", true).unwrap().check_arity(cx, call, args)?;
    let utf8 = cx.locale().utf8;
    cx.build(out, |cx| {
        fn lgl(cx: &Cx, b: bool) -> Result<Ref<'_>, robigo_rtcore::NeedGc> {
            Ok(cx.alloc_copy(&[Lgl(b as i32)])?.finish().into())
        }
        let mut ans = cx.alloc_vector::<Ref<'_>>(4)?;
        ans.set(0, lgl(cx, utf8)?);
        ans.set(1, lgl(cx, utf8)?);
        ans.set(2, lgl(cx, false)?);
        ans.set(3, crate::coerce::mk_string(cx, if utf8 { b"UTF-8" } else { b"ANSI_X3.4-1968" })?);
        let mut names = cx.alloc_vector::<Char<'_>>(4)?;
        for (i, n) in [&b"MBCS"[..], b"UTF-8", b"Latin-1", b"codeset"].iter().enumerate() {
            names.set(i, cx.mk_char(n, Encoding::Native)?);
        }
        let mut a = Attrs::new();
        a.install(Attr { tag: cx.install(b"names")?, value: names.finish().into() });
        a.apply(cx, &mut ans)?;
        Ok(ans.finish().into())
    })
}
