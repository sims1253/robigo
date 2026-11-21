//! sysutils.c's `.Internal(Sys.glob(paths, dirmark))` (`do_glob`), over
//! `glob(3)`.

use robigo_rtcore::clib::{Glob, GlobRes};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Encoding, Out, RResult, Root};

/// `.Internal(Sys.glob(paths, dirmark))`: the paths matching each pattern
/// (NA ones skipped), all patterns' matches in one vector, each pattern's
/// sorted by glob; with `dirmark`, a `/` after each directory.
// R: sysutils.c:do_glob
pub fn do_glob(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("Sys.glob", true).check_arity(cx, call, args)?;
    let x = args.get(cx, 0);
    if x.sexptype() != STRSXP {
        return Err(cx.error("invalid 'paths' argument"));
    }
    let n = crate::coerce::xlength(x);
    if n == 0 {
        return cx.build(out, |cx| Ok(cx.alloc_vector::<Char<'_>>(0)?.finish().into()));
    }
    let dirmark = crate::coerce::vector::as_logical(cx, args.get(cx, 1));
    if dirmark == i32::MIN {
        return Err(cx.error("invalid 'dirmark' argument"));
    }
    if x.is_package_altrep() {
        return Err(crate::util::not_yet(cx, Some(call), "Sys.glob() of a package ALTREP vector"));
    }
    let x = args.root(0);
    let mut globbuf = Glob::new();
    for i in 0..n {
        let na = {
            let xv = x.bind(cx);
            let el = xv.as_vector::<Char<'_>>().expect("robigo: checked: character").get(i);
            el == cx.na_string()
        };
        if na {
            continue;
        }
        let el = crate::text::native_elt(cx, x, i)?;
        match globbuf.append(&el, dirmark != 0) {
            GlobRes::Ok => {}
            GlobRes::Aborted => {
                let msg = format!("read error on '{}'", String::from_utf8_lossy(&el));
                cx.warning(&msg)?;
            }
            GlobRes::NoSpace => return Err(cx.error("internal out-of-memory condition")),
        }
    }
    let paths = globbuf.paths();
    drop(globbuf);
    cx.build(out, |cx| {
        let mut ans = cx.alloc_vector::<Char<'_>>(paths.len())?;
        for (i, p) in paths.iter().enumerate() {
            ans.set(i, cx.mk_char(p, Encoding::Native)?);
        }
        Ok(ans.finish().into())
    })
}
