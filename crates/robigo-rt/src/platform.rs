//! platform.c's `Init_R_Platform`: `.Platform` in base, the list GNU R
//! makes at startup (`dyn.load`'s definition reads `.Platform$OS.type`).
//! The values are a unix build's under Rscript, as GNU R 4.6.2's oracle
//! reports them. `pkgType` is the environment variable
//! `R_PLATFORM_PKGTYPE` when it is set and not empty (its bytes, as `getenv`
//! gives them; GNU R reads it since 4.6.0, so that builds can set it in
//! their `Renviron`), else the configure value `PLATFORM_PKGTYPE`, which a
//! unix build leaves unset: `"source"` (platform.c).

use crate::connections::R_PATH_MAX;
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
                let p = crate::connections::r_expand_file_name(cx, b)?;
                let path = <std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(&p);
                // glibc's remove(): unlink, and rmdir only when unlink
                // failed with EISDIR (the errno GNU R then reports is
                // rmdir's)
                const EISDIR: i32 = 21;
                match std::fs::remove_file(path)
                    .or_else(|e| if e.raw_os_error() == Some(EISDIR) { std::fs::remove_dir(path) } else { Err(e) })
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

/// `.Internal(R.home())`: `R_HomeDir()`, the environment variable
/// `R_HOME` (GNU R's front end sets it).
// R: platform.c:do_Rhome, sysutils.c:R_HomeDir
pub fn do_rhome(
    cx: &mut Cx,
    call: robigo_rtcore::Root<'_>,
    args: robigo_rtcore::Args<'_>,
    _env: robigo_rtcore::Root<'_>,
    out: robigo_rtcore::Out<'_>,
) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "R.home", 0, true)?;
    let Some(home) = std::env::var_os("R_HOME") else {
        return Err(cx.error("unable to determine R home location"));
    };
    let b = std::os::unix::ffi::OsStrExt::as_bytes(home.as_os_str()).to_vec();
    cx.build(out, |cx| cx.mk_string(&b, Encoding::Native))
}

/// `.Internal(dir.exists(paths))`: whether each path names a directory
/// (`stat`, so a link to one counts). NA is FALSE; so is a name iconv
/// cannot translate, after `translateCharFP2`'s warning; a "bytes" name
/// is its error.
// R: platform.c:do_direxists, sysutils.c:translateCharFP2
pub fn do_direxists(
    cx: &mut Cx,
    call: robigo_rtcore::Root<'_>,
    args: robigo_rtcore::Args<'_>,
    _env: robigo_rtcore::Root<'_>,
    out: robigo_rtcore::Out<'_>,
) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "dir.exists", 1, true)?;
    let fn_ = args.root(0);
    let n = match fn_.bind(cx).view() {
        robigo_rtcore::View::Str(v) => v.len(),
        _ => return Err(cx.error("invalid filename argument")),
    };
    let mut ans = Vec::with_capacity(n);
    for i in 0..n {
        // (translateCharFP2 of NA is "NA": nothing to warn about)
        let is_dir = match translate_fp2(cx, fn_, i)? {
            Some(b) => {
                let p = crate::connections::r_expand_file_name(cx, &b)?;
                std::fs::metadata(<std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(&p))
                    .is_ok_and(|m| m.is_dir())
            }
            None => false,
        };
        ans.push(robigo_rtcore::Lgl(is_dir as i32));
    }
    cx.build(out, |cx| Ok(cx.alloc_copy(&ans)?.finish().into()))
}

/// `.Internal(dir.create(path, showWarnings, recursive, mode))`: make the
/// directory `path` (its parents too with `recursive`), with `mode` (as
/// `mkdir` takes it, under the umask); TRUE if the last `mkdir` succeeded.
/// With `showWarnings`, an existing directory and a failure are warnings.
// R: platform.c:do_dircreate
pub fn do_dircreate(
    cx: &mut Cx,
    call: robigo_rtcore::Root<'_>,
    args: robigo_rtcore::Args<'_>,
    _env: robigo_rtcore::Root<'_>,
    out: robigo_rtcore::Out<'_>,
) -> RResult<()> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::DirBuilderExt;
    crate::prims::check_arity(cx, call, args.len(), "dir.create", 4, true)?;
    let path = args.root(0);
    let pv = path.bind(cx);
    if pv.sexptype() != robigo_heap::types::STRSXP || pv.is_package_altrep() || pv.len() != Some(1) {
        if pv.sexptype() == robigo_heap::types::STRSXP && pv.is_package_altrep() {
            return Err(robigo_rtbuiltins::util::not_yet(cx, Some(call), "dir.create() of a package ALTREP vector"));
        }
        return Err(cx.error("invalid 'path' argument"));
    }
    let lgl =
        |cx: &mut Cx, b: bool| cx.build(out, |cx| Ok(cx.alloc_copy(&[robigo_rtcore::Lgl(b as i32)])?.finish().into()));
    if pv.as_vector::<Char<'_>>().unwrap().get(0) == cx.na_string() {
        return lgl(cx, false);
    }
    const NA: i32 = i32::MIN;
    let show = robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 1));
    let show = show != NA && show != 0;
    let recursive = robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 2));
    let recursive = recursive != NA && recursive != 0;
    let mode = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(3))?;
    // (mode == NA_LOGICAL: NA_INTEGER, the same)
    let mode = if mode == NA { 0o777 } else { mode as u32 };
    // translateCharFP: its error for a name that cannot be translated
    let name = {
        let c = path.bind(cx).as_vector::<Char<'_>>().unwrap().get(0);
        robigo_rtbuiltins::text::to_native_fp(robigo_rtbuiltins::coerce::text(c), cx.locale()).map(|b| b.into_owned())
    };
    let name = name.map_err(|m| cx.error(&m))?;
    let mut dir = crate::connections::r_expand_file_name(cx, &name)?;
    if dir.is_empty() {
        return Err(cx.error("zero-length 'path' argument"));
    }
    // remove trailing slashes
    while dir.len() > 1 && dir.last() == Some(&b'/') {
        dir.pop();
    }
    let os = |b: &[u8]| std::ffi::OsStr::from_bytes(b).to_owned();
    let mkdir = |b: &[u8]| std::fs::DirBuilder::new().mode(mode).create(os(b));
    let errno = |e: &std::io::Error| e.raw_os_error().unwrap_or(0);
    // Linux's <errno.h> (robigo-rt is Linux with glibc)
    const ENOENT: i32 = 2;
    const EEXIST: i32 = 17;
    const ENOTDIR: i32 = 20;
    const EROFS: i32 = 30;
    const ENOSYS: i32 = 38;
    // (res, serrno, gone to `end` past the last mkdir)
    let mut failed: Option<std::io::Error> = None;
    let mut ended = false;
    // the name the warnings give: `dir` as the C leaves it (cut at the
    // component where the recursive walk stopped)
    let mut shown_len = dir.len();
    if recursive {
        let mut k = 0;
        while let Some(off) = dir[k + 1..].iter().position(|&b| b == b'/') {
            k = k + 1 + off;
            let prefix = &dir[..k];
            match std::fs::metadata(os(prefix)) {
                Ok(m) => {
                    if !m.is_dir() {
                        // file already exists but is not a directory
                        failed = Some(std::io::Error::from_raw_os_error(ENOTDIR));
                        ended = true;
                        shown_len = k;
                        break;
                    }
                }
                Err(e) if errno(&e) != ENOENT => {
                    failed = Some(e);
                    ended = true;
                    shown_len = k;
                    break;
                }
                Err(_) => {
                    if let Err(e) = mkdir(prefix) {
                        // Solaris 10 returns ENOSYS on automount, PR#13834;
                        // EROFS is allowed by POSIX, so we skip that too
                        if !matches!(errno(&e), EEXIST | ENOSYS | EROFS) {
                            failed = Some(e);
                            ended = true;
                            shown_len = k;
                            break;
                        }
                    }
                }
            }
        }
    }
    let shown = String::from_utf8_lossy(&dir[..shown_len]).into_owned();
    if !ended {
        if let Err(e) = mkdir(&dir) {
            if show && errno(&e) == EEXIST {
                cx.warning(&format!("'{shown}' already exists"))?;
            }
            failed = Some(e);
        }
    }
    // end:
    if let Some(e) = &failed
        && show
        && errno(e) != EEXIST
    {
        let msg = format!("cannot create dir '{shown}', reason '{}'", crate::connections::strerror(e));
        cx.warning(&msg)?;
    }
    lgl(cx, failed.is_none())
}

/// `translateCharFP2(s)` of element `i` of a character vector: `None` for NA
/// and, after its warning, for a name iconv cannot translate; a "bytes"
/// name is its error.
// R: sysutils.c:translateCharFP2
fn translate_fp2(cx: &mut Cx, x: robigo_rtcore::Root<'_>, i: usize) -> RResult<Option<Vec<u8>>> {
    let loc = robigo_rtbuiltins::text::ctype(cx.locale());
    let (tr, na) = {
        let c = x.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
        let tr = robigo_text::translate_char2(robigo_rtbuiltins::coerce::text(c), loc).map(|b| b.into_owned());
        (tr, c == cx.na_string())
    };
    if na {
        return Ok(None);
    }
    match tr {
        Ok(b) => Ok(Some(b)),
        Err(robigo_text::Untranslatable::Bytes) => {
            Err(cx.error("translating strings with \"bytes\" encoding is not allowed"))
        }
        Err(robigo_text::Untranslatable::Failed(m)) => {
            cx.warning(&format!("unable to translate '{m}' to native encoding"))?;
            Ok(None)
        }
    }
}

/// `stat(path) == 0`.
fn stat_ok(path: &[u8]) -> bool {
    std::fs::metadata(<std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(path)).is_ok()
}

/// `R_IsDirPath(path)`: `stat` says a directory (a link to one counts).
// R: connections.c:R_IsDirPath
fn is_dir_path(path: &[u8]) -> bool {
    std::fs::metadata(<std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(path)).is_ok_and(|m| m.is_dir())
}

/// `.Internal(file.exists(file))`: whether each file exists (`stat` of its
/// expanded name). NA is FALSE, and so is a name iconv cannot translate
/// (after the warning) or one longer than `R_PATH_MAX`.
// R: platform.c:do_fileexists, R_FileExists
pub fn do_fileexists(
    cx: &mut Cx,
    call: robigo_rtcore::Root<'_>,
    args: robigo_rtcore::Args<'_>,
    _env: robigo_rtcore::Root<'_>,
    out: robigo_rtcore::Out<'_>,
) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "file.exists", 1, true)?;
    let file = args.root(0);
    let n = match file.bind(cx).view() {
        robigo_rtcore::View::Str(v) => v.len(),
        _ => return Err(cx.error("invalid 'file' argument")),
    };
    let mut ans = Vec::with_capacity(n);
    for i in 0..n {
        // (R_CheckUserInterrupt every 16: robigo-rt's interrupts are S3x's)
        let ok = match translate_fp2(cx, file, i)? {
            // documented to report FALSE for paths too long after expansion
            Some(p) if p.len() <= R_PATH_MAX => stat_ok(&crate::connections::r_expand_file_name(cx, &p)?),
            _ => false,
        };
        ans.push(robigo_rtcore::Lgl(ok as i32));
    }
    cx.build(out, |cx| Ok(cx.alloc_copy(&ans)?.finish().into()))
}

/// What `list_files` reads: the walk's options and the pattern.
struct ListFiles<'a> {
    allfiles: bool,
    recursive: bool,
    idirs: bool,
    allowdots: bool,
    reg: Option<&'a robigo_text::tre::Regex>,
    utf8: bool,
}

impl ListFiles<'_> {
    /// `tre_regexec(reg, name, 0, NULL, 0) == 0` (no pattern matches all).
    /// In a UTF-8 locale GNU R matches the name as a multibyte string, which
    /// TRE's backtracking matcher (back references) walks with a position
    /// it does not restore on backing up; robigo-text has not ported that
    /// path, so such a pattern is refused rather than answered differently.
    fn matches(&self, cx: &mut Cx, name: &[u8]) -> RResult<bool> {
        let Some(r) = self.reg else { return Ok(true) };
        let mut m = [];
        let hit = if self.utf8 {
            if r.have_backrefs() {
                return Err(cx.error(
                    "robigo: list.files() with a back reference in 'pattern' in a UTF-8 locale \
                     (TRE's backtracking matcher on a multibyte string) is not implemented yet",
                ));
            }
            r.exec_mbs(name, &mut m, 0)
        } else {
            r.exec_bytes(name, &mut m, 0)
        };
        match hit {
            Ok(h) => Ok(h),
            // a status (REG_NOMATCH, REG_ESPACE): not 0, so no match
            Err(robigo_text::tre::Error::Status(_)) => Ok(false),
            // R_assert's error (TRE's `assert`), or what robigo has not
            // ported
            Err(e) => Err(cx.error(&e.message())),
        }
    }

    /// `list_files(pb, offset, len, ..., dir)`: the entries of `dir`, the
    /// directory `pb[..len]` names (it ends in a separator; `dir` was opened
    /// before the separator was added, as GNU R opens it), with `.` and
    /// `..` as `readdir` gives them (their order does not matter: the
    /// result is sorted); a match is added as `pb[offset..]`.
    fn list(
        &self,
        cx: &mut Cx,
        pb: &mut Vec<u8>,
        offset: usize,
        len: usize,
        ans: &mut Vec<Vec<u8>>,
        rd: std::fs::ReadDir,
    ) -> RResult<()> {
        let mut names: Vec<Vec<u8>> = vec![b".".to_vec(), b"..".to_vec()];
        names.extend(
            rd.filter_map(|e| e.ok())
                .map(|e| std::os::unix::ffi::OsStrExt::as_bytes(e.file_name().as_os_str()).to_vec()),
        );
        for name in names {
            // R_HiddenFile: a name starting with "."
            if !(self.allfiles || name.first() != Some(&b'.')) {
                continue;
            }
            // path_buffer_append
            pb.truncate(len);
            pb.extend_from_slice(&name);
            if pb.len() + 1 > R_PATH_MAX {
                cx.warning("over-long path")?;
            }
            let not_dot = name != b"." && name != b"..";
            if self.recursive && is_dir_path(pb) {
                if not_dot {
                    if self.idirs && self.matches(cx, &name)? {
                        ans.push(pb[offset..].to_vec());
                    }
                    // R_opendir(pb->data), then the terminator becomes a
                    // separator
                    if let Ok(newdir) =
                        std::fs::read_dir(<std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(pb))
                    {
                        pb.push(b'/');
                        let newlen = pb.len();
                        self.list(cx, pb, offset, newlen, ans, newdir)?;
                    }
                }
                continue;
            }
            if (not_dot || self.allowdots) && self.matches(cx, &name)? {
                ans.push(pb[offset..].to_vec());
            }
        }
        Ok(())
    }
}

/// `.Internal(list.files(path, pattern, all.files, full.names, recursive,
/// ignore.case, include.dirs, no.., fixed))`: the names in the directories
/// `path` (matching `pattern`, a TRE extended regex), sorted.
// R: platform.c:do_listfiles, search_setup, list_files
pub fn do_listfiles(
    cx: &mut Cx,
    call: robigo_rtcore::Root<'_>,
    args: robigo_rtcore::Args<'_>,
    _env: robigo_rtcore::Root<'_>,
    out: robigo_rtcore::Out<'_>,
) -> RResult<()> {
    use robigo_rtbuiltins::util::{as_bool2, as_logical};
    use robigo_rtcore::View;
    use robigo_rtcore::types::{NILSXP, STRSXP};
    const NA_LOGICAL: i32 = i32::MIN;
    crate::prims::check_arity(cx, call, args.len(), "list.files", 9, true)?;
    let d = args.root(0);
    let nd = match d.bind(cx).view() {
        View::Str(v) => v.len(),
        _ => return Err(cx.error("invalid 'path' argument")),
    };
    let p = args.get(cx, 1);
    let pattern = match p.view() {
        View::Str(v) => !v.is_empty() && v.get(0) != cx.na_string(),
        _ => false,
    };
    if !pattern && p.sexptype() != NILSXP && !(p.sexptype() == STRSXP && p.len() == Some(0)) {
        return Err(cx.error("invalid 'pattern' argument"));
    }
    let allfiles = as_bool2(cx, call, args.root(2))?;
    let fullnames = as_logical(cx, args.get(cx, 3));
    if fullnames == NA_LOGICAL {
        return Err(cx.error("invalid 'full.names' argument"));
    }
    let recursive = as_bool2(cx, call, args.root(4))?;
    let igcase = as_logical(cx, args.get(cx, 5));
    if igcase == NA_LOGICAL {
        return Err(cx.error("invalid 'ignore.case' argument"));
    }
    let idirs = as_bool2(cx, call, args.root(6))?;
    let nodots = as_logical(cx, args.get(cx, 7));
    if nodots == NA_LOGICAL {
        return Err(cx.error("invalid 'no..' argument"));
    }
    // (GNU R checks `nodots` again here, not `fixed`: an NA `fixed` is
    // read as true)
    let fixed = as_logical(cx, args.get(cx, 8));
    let mut flags = robigo_text::tre::REG_EXTENDED;
    if igcase != 0 {
        flags |= robigo_text::tre::REG_ICASE;
    }
    if fixed != 0 {
        flags |= robigo_text::tre::REG_LITERAL;
    }
    let loc = cx.locale();
    let tloc = robigo_rtbuiltins::text::ctype(loc);
    let reg = if pattern {
        // tre_regcomp(translateChar(pattern)): the multibyte pattern read as
        // wide characters in the locale
        let pat = {
            let c = args.get(cx, 1).as_vector::<Char<'_>>().unwrap().get(0);
            robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), loc).map(|b| b.into_owned())
        };
        let Ok(pat) = pat else {
            return Err(cx.error("translating strings with \"bytes\" encoding is not allowed"));
        };
        let wide: Option<Vec<i32>> = if loc.utf8 {
            std::str::from_utf8(&pat).ok().map(|s| s.chars().map(|c| c as i32).collect())
        } else {
            Some(pat.iter().map(|&b| b as i32).collect())
        };
        match wide.map(|w| robigo_text::tre::Regex::new(&w, flags, tloc)) {
            Some(Ok(r)) => Some(r),
            _ => return Err(cx.error("invalid 'pattern' regular expression")),
        }
    } else {
        None
    };
    let lf = ListFiles { allfiles, recursive, idirs, allowdots: nodots == 0, reg: reg.as_ref(), utf8: loc.utf8 };
    let mut ans: Vec<Vec<u8>> = Vec::new();
    for i in 0..nd {
        // search_setup: the expanded path, opened, then a separator
        let Some(path) = translate_fp2(cx, d, i)? else { continue };
        let mut pb = crate::connections::r_expand_file_name(cx, &path)?;
        let Ok(dir) = std::fs::read_dir(<std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(&pb)) else {
            continue;
        };
        pb.push(b'/');
        let len = pb.len();
        lf.list(cx, &mut pb, if fullnames != 0 { 0 } else { len }, len, &mut ans, dir)?;
    }
    if let Err(e) = robigo_rtbuiltins::sort::ssort_native(&mut ans, loc) {
        let d = e.diag(Some(call));
        return Err(cx.raise(d));
    }
    cx.build(out, |cx| {
        let mut v = cx.alloc_vector::<Char<'_>>(ans.len())?;
        for (i, a) in ans.iter().enumerate() {
            v.set(i, cx.mk_char(a, Encoding::Native)?);
        }
        Ok(v.finish().into())
    })
}
