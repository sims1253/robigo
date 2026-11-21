//! sys-unix.c's `.Internal(Sys.info())` (`do_sysinfo`): `uname`, the login
//! name and the real and effective users' names.

use crate::attrib::{Attr, Attrs};
use robigo_rtcore::clib;
use robigo_rtcore::{Args, Char, Cx, Encoding, Out, RResult, RSym, Root};

/// `.Internal(Sys.info())`: `sysname`, `release`, `version`, `nodename`,
/// `machine`, `login`, `user` and `effective_user`, each "unknown" when the
/// C library has none; NULL when `uname` fails.
// R: sys-unix.c:do_sysinfo
pub fn do_sysinfo(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("Sys.info", true).check_arity(cx, call, args)?;
    let Some(u) = clib::uname() else {
        return cx.build(out, |cx| Ok(cx.nil()));
    };
    let unknown = || b"unknown".to_vec();
    let [sysname, release, version, nodename, machine] = u;
    let vals = [
        sysname,
        release,
        version,
        nodename,
        machine,
        clib::login_name().unwrap_or_else(unknown),
        clib::passwd_name(clib::uid()).unwrap_or_else(unknown),
        clib::passwd_name(clib::euid()).unwrap_or_else(unknown),
    ];
    const NAMES: [&[u8]; 8] =
        [b"sysname", b"release", b"version", b"nodename", b"machine", b"login", b"user", b"effective_user"];
    cx.build(out, |cx| {
        let mut ans = cx.alloc_vector::<Char<'_>>(8)?;
        for (i, v) in vals.iter().enumerate() {
            ans.set(i, cx.mk_char(v, Encoding::Native)?);
        }
        let mut names = cx.alloc_vector::<Char<'_>>(8)?;
        for (i, n) in NAMES.iter().enumerate() {
            names.set(i, cx.mk_char(n, Encoding::Native)?);
        }
        let mut a = Attrs::new();
        a.install(Attr { tag: cx.sym(RSym::Names), value: names.finish().into() });
        a.apply(cx, &mut ans)?;
        Ok(ans.finish().into())
    })
}
