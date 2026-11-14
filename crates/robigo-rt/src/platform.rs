//! platform.c's `Init_R_Platform`: `.Platform` in base, the list GNU R
//! makes at startup (`dyn.load`'s definition reads `.Platform$OS.type`).
//! The values are a unix build's under Rscript, as GNU R 4.6.2's oracle
//! reports them.

use robigo_rtcore::{Char, Cx, Encoding, RResult};

/// Define `.Platform` in the base environment.
// R: platform.c:Init_R_Platform
pub fn init(cx: &mut Cx) -> RResult<()> {
    let pkg_type = std::env::var("R_PLATFORM_PKGTYPE").ok().filter(|v| !v.is_empty());
    let fields: [(&[u8], &[u8]); 8] = [
        (b"OS.type", b"unix"),
        (b"file.sep", b"/"),
        (b"dynlib.ext", b".so"),
        // R_GUIType: "X11" unless --gui says otherwise (unix/system.c)
        (b"GUI", b"X11"),
        (b"endian", if cfg!(target_endian = "big") { b"big" } else { b"little" }),
        (b"pkgType", pkg_type.as_deref().map_or(b"source".as_slice(), str::as_bytes)),
        (b"path.sep", b":"),
        (b"r_arch", b""),
    ];
    cx.scope(|cx, s| {
        let value = s.out(cx);
        let v = value.root();
        cx.build(value, |cx| {
            let mut l = cx.alloc_vector::<robigo_rtcore::Ref<'_>>(fields.len())?;
            let mut names = cx.alloc_vector::<Char<'_>>(fields.len())?;
            for (i, (name, val)) in fields.iter().enumerate() {
                names.set(i, cx.mk_char(name, Encoding::Native)?);
                let mut e = cx.alloc_vector::<Char<'_>>(1)?;
                e.set(0, cx.mk_char(val, Encoding::Native)?);
                l.set(i, e.finish().into());
            }
            let attr = cx.cons_tagged(names.finish().into(), cx.nil(), cx.install(b"names")?)?;
            l.set_attrib(attr);
            Ok(l.finish().into())
        })?;
        let sym = cx.install_rooted(s, b".Platform")?;
        let base = s.root(cx, cx.base_env());
        cx.define_var(sym, v, base)
    })
}
