//! platform.c's `Init_R_Platform`: `.Platform` in base, the list GNU R
//! makes at startup (`dyn.load`'s definition reads `.Platform$OS.type`).
//! The values are a unix build's under Rscript, as GNU R 4.6.2's oracle
//! reports them. `pkgType` is the environment variable
//! `R_PLATFORM_PKGTYPE` when it is set and not empty (its bytes, as `getenv`
//! gives them; GNU R reads it since 4.6.0, so that builds can set it in
//! their `Renviron`), else the configure value `PLATFORM_PKGTYPE`, which a
//! unix build leaves unset: `"source"` (platform.c).

use robigo_numerics::ldouble::LDouble;
use robigo_numerics::machar::machar;
use robigo_rtcore::{Char, Cx, Encoding, RResult};

/// Define `.Platform` in the base environment.
// R: platform.c:Init_R_Platform
pub fn init(cx: &mut Cx) -> RResult<()> {
    // R: platform.c:Init_R_Platform, getenv("R_PLATFORM_PKGTYPE") over the
    // configure default (unix: "source")
    let pkg_type = std::env::var_os("R_PLATFORM_PKGTYPE").filter(|v| !v.is_empty());
    let fields: [(&[u8], &[u8]); 8] = [
        (b"OS.type", b"unix"),
        (b"file.sep", b"/"),
        (b"dynlib.ext", b".so"),
        // R_GUIType: "X11" unless --gui says otherwise (unix/system.c)
        (b"GUI", b"X11"),
        (b"endian", if cfg!(target_endian = "big") { b"big" } else { b"little" }),
        (b"pkgType", pkg_type.as_deref().map_or(b"source".as_slice(), std::os::unix::ffi::OsStrExt::as_bytes)),
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

/// One `.Machine` entry: a double or an integer scalar.
enum MachineValue {
    Real(f64),
    Int(i32),
}

/// Define `.Machine` in the base environment: what `machar` finds for
/// `double` and `long double`, and the C types' sizes on this target
/// (LP64: `long` 8 bytes, `long double` 16, `time_t` 8).
// R: machine.c:Init_R_Machine
pub fn init_machine(cx: &mut Cx) -> RResult<()> {
    use MachineValue::{Int, Real};
    let d = machar::<f64>();
    let ld = machar::<LDouble>();
    let mut fields: Vec<(&[u8], MachineValue)> = vec![
        (b"double.eps", Real(d.eps)),
        (b"double.neg.eps", Real(d.epsneg)),
        (b"double.xmin", Real(d.xmin)),
        (b"double.xmax", Real(d.xmax)),
        (b"double.base", Int(d.ibeta)),
        (b"double.digits", Int(d.it)),
        (b"double.rounding", Int(d.irnd)),
        (b"double.guard", Int(d.ngrd)),
        (b"double.ulp.digits", Int(d.machep)),
        (b"double.neg.ulp.digits", Int(d.negep)),
        (b"double.exponent", Int(d.iexp)),
        (b"double.min.exp", Int(d.minexp)),
        (b"double.max.exp", Int(d.maxexp)),
        (b"integer.max", Int(i32::MAX)),
        (b"sizeof.long", Int(size_of::<std::ffi::c_long>() as i32)),
        (b"sizeof.longlong", Int(size_of::<std::ffi::c_longlong>() as i32)),
        (b"sizeof.longdouble", Int(size_of::<LDouble>() as i32)),
        (b"sizeof.pointer", Int(size_of::<*const u8>() as i32)),
        (b"sizeof.time_t", Int(size_of::<i64>() as i32)),
    ];
    // (sizeof(LDOUBLE) > sizeof(double): the long double entries)
    fields.extend([
        (b"longdouble.eps".as_slice(), Real(ld.eps.to_f64())),
        (b"longdouble.neg.eps", Real(ld.epsneg.to_f64())),
        (b"longdouble.digits", Int(ld.it)),
        (b"longdouble.rounding", Int(ld.irnd)),
        (b"longdouble.guard", Int(ld.ngrd)),
        (b"longdouble.ulp.digits", Int(ld.machep)),
        (b"longdouble.neg.ulp.digits", Int(ld.negep)),
        (b"longdouble.exponent", Int(ld.iexp)),
        (b"longdouble.min.exp", Int(ld.minexp)),
        (b"longdouble.max.exp", Int(ld.maxexp)),
    ]);
    cx.scope(|cx, s| {
        let value = s.out(cx);
        let v = value.root();
        cx.build(value, |cx| {
            let mut l = cx.alloc_vector::<robigo_rtcore::Ref<'_>>(fields.len())?;
            let mut names = cx.alloc_vector::<Char<'_>>(fields.len())?;
            for (i, (name, val)) in fields.iter().enumerate() {
                names.set(i, cx.mk_char(name, Encoding::Native)?);
                let e = match *val {
                    Real(x) => cx.alloc_copy(&[x])?.finish().into(),
                    Int(x) => cx.alloc_copy(&[x])?.finish().into(),
                };
                l.set(i, e);
            }
            let attr = cx.cons_tagged(names.finish().into(), cx.nil(), cx.install(b"names")?)?;
            l.set_attrib(attr);
            Ok(l.finish().into())
        })?;
        let sym = cx.install_rooted(s, b".Machine")?;
        let base = s.root(cx, cx.base_env());
        cx.define_var(sym, v, base)
    })
}

/// `file.remove(x)`: remove each file, TRUE where that worked; GNU R's
/// warning for each that did not.
// R: platform.c:do_fileremove
pub fn do_fileremove(
    cx: &mut Cx,
    call: robigo_rtcore::Root<'_>,
    args: robigo_rtcore::Args<'_>,
    _env: robigo_rtcore::Root<'_>,
    out: robigo_rtcore::Out<'_>,
) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "file.remove", 1, true)?;
    let names: Option<Vec<Option<Vec<u8>>>> = match args.get(cx, 0).view() {
        robigo_rtcore::View::Str(v) => Some(
            (0..v.len())
                .map(|i| {
                    let c = v.get(i);
                    (c != cx.na_string()).then(|| {
                        robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), cx.locale())
                            .map(|b| b.into_owned())
                            .unwrap_or_else(|_| c.bytes().to_vec())
                    })
                })
                .collect(),
        ),
        _ => None,
    };
    let Some(names) = names else { return Err(cx.error("invalid first filename")) };
    let mut ans = Vec::with_capacity(names.len());
    for n in &names {
        let ok = match n {
            Some(b) => {
                let p = crate::connections::expand_file_name(b);
                let path = <std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(&p);
                match std::fs::remove_file(path)
                    .or_else(|e| if std::path::Path::new(path).is_dir() { std::fs::remove_dir(path) } else { Err(e) })
                {
                    Ok(()) => true,
                    Err(e) => {
                        let shown = String::from_utf8_lossy(b).into_owned();
                        cx.warning(&format!(
                            "cannot remove file '{shown}', reason '{}'",
                            crate::connections::strerror(&e)
                        ))?;
                        false
                    }
                }
            }
            None => false,
        };
        ans.push(robigo_rtcore::Lgl(ok as i32));
    }
    cx.build(out, |cx| Ok(cx.alloc_copy(&ans)?.finish().into()))
}

/// `Sys.getpid()`.
// R: sysutils.c:do_sysgetpid
pub fn do_sysgetpid(
    cx: &mut Cx,
    call: robigo_rtcore::Root<'_>,
    args: robigo_rtcore::Args<'_>,
    _env: robigo_rtcore::Root<'_>,
    out: robigo_rtcore::Out<'_>,
) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "Sys.getpid", 0, true)?;
    cx.build_scalar(out, std::process::id() as i32)
}
