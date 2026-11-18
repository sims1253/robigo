//! The lazy-load and save/load internals GNU R's build reaches (L4):
//! builtin.c's `makeLazy` (each lazy-load binding a promise of the fetch),
//! saveload.c's `loadInfoFromConn2` (a saved file's serialization header)
//! and the file pair `save.to.file` / `load.from.file` (cmp.R's
//! `cmpfile`/`loadcmp`), in formats 2 and 3. GNU R's version 1 and older
//! formats say they are not implemented yet.

use super::{Format, read, write};
use crate::connections;
use robigo_heap::types::*;
use robigo_rtcore::{Args, Char, Cx, Out, RResult, Ref, Root};

/// `makeLazy(names, values, expr, eenv, aenv)`: for each name, `expr` with
/// its second element the value of `values[[i]]` evaluated in `eenv`,
/// bound in `aenv` as a promise to be evaluated in `eenv`.
// R: builtin.c:do_makelazy
pub fn do_makelazy(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "makeLazy", 5, true)?;
    let names = args.root(0);
    if names.bind(cx).sexptype() != STRSXP {
        return Err(cx.error("invalid first argument"));
    }
    let (values, expr, eenv, aenv) = (args.root(1), args.root(2), args.root(3), args.root(4));
    if eenv.bind(cx).sexptype() != ENVSXP {
        return Err(cx.error("invalid 'eval.env' argument"));
    }
    if aenv.bind(cx).sexptype() != ENVSXP {
        return Err(cx.error("invalid 'assign.env' argument"));
    }
    let n = names.bind(cx).len().unwrap_or(0);
    for i in 0..n {
        cx.scope(|cx, s| {
            let name = robigo_rtbuiltins::envir::install_tr(cx, s, names, i)?;
            // VECTOR_ELT(values, i)
            let v = values.bind(cx);
            let Some(elt) = v.as_vector::<Ref<'_>>().filter(|_| matches!(v.sexptype(), VECSXP | EXPRSXP)) else {
                let t = robigo_rtbuiltins::util::type2char(v.sexptype());
                return Err(cx.error(&format!("VECTOR_ELT() can only be applied to a 'list', not a '{t}'")));
            };
            let e = s.root(cx, elt.get(i));
            let vo = s.out(cx);
            let val = vo.root();
            crate::eval::eval(cx, e, eenv, vo)?;
            let d = s.out(cx);
            let expr0 = d.root();
            cx.duplicate(expr, d)?;
            // SETCAR(CDR(expr0), val)
            let Some(cell) = expr0.bind(cx).as_cons().map(|c| c.cdr()).filter(|c| c.as_cons().is_some()) else {
                return Err(
                    cx.error("robigo: makeLazy() of an expression with no second element is not implemented yet")
                );
            };
            let cell = s.root(cx, cell);
            cx.set_car(cell, val);
            let p = s.out(cx);
            let prom = p.root();
            cx.build(p, |cx| cx.mk_promise(expr0.bind(cx), eenv.bind(cx)))?;
            cx.define_var(name, prom, aenv)
        })?;
    }
    out.set(cx, cx.nil());
    Ok(())
}

/// The magic numbers a version 2 or 3 file starts with.
const MAGICS: [&[u8; 5]; 6] = [b"RDA2\n", b"RDB2\n", b"RDX2\n", b"RDA3\n", b"RDB3\n", b"RDX3\n"];

/// `loadInfoFromConn2(con)`: the serialization header of what `con` holds
/// after its magic number (`serializeInfo`'s list).
// R: saveload.c:do_loadFromConn2 (PRIMVAL 1)
pub fn do_load_info_from_conn2(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "loadInfoFromConn2", 1, true)?;
    load_from_conn2(cx, args, env, out, true)
}

/// `loadFromConn2(con, envir, verbose)`: the variables a saved file holds,
/// bound in `envir`; their names.
// R: saveload.c:do_loadFromConn2 (PRIMVAL 0)
pub fn do_load_from_conn2(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "loadFromConn2", 3, true)?;
    load_from_conn2(cx, args, env, out, false)
}

/// `do_loadFromConn2`'s two ops: the header (`info`) or the variables.
// R: saveload.c:do_loadFromConn2
fn load_from_conn2(cx: &mut Cx, args: Args<'_>, _env: Root<'_>, out: Out<'_>, info: bool) -> RResult<()> {
    let n = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(0))?;
    let wasopen = connections::flags(cx, n)?.isopen;
    if !wasopen && !connections::open_with_mode(cx, n, "rb")? {
        return Err(cx.error("cannot open the connection"));
    }
    // an error closes a connection this opened (the context's cend)
    let r = (|| -> RResult<()> {
        let f = connections::flags(cx, n)?;
        if !f.canread {
            return Err(cx.error("connection not open for reading"));
        }
        if f.text {
            return Err(cx.error("can only load() from a binary connection"));
        }
        if !info {
            match args.get(cx, 1).sexptype() {
                NILSXP => return Err(cx.error("use of NULL environment is defunct")),
                ENVSXP => {}
                _ => return Err(cx.error("invalid 'envir' argument")),
            }
        }
        // check magic
        let mut buf = [0u8; 5];
        let count = connections::read_upto(cx, n, &mut buf)?;
        if count == 0 {
            return Err(cx.error("no input is available"));
        }
        if !MAGICS.iter().any(|m| **m == buf) {
            return Err(
                cx.error("the input does not start with a magic number compatible with loading from a connection")
            );
        }
        let st = super::conn_in_stream(cx, n)?;
        if info {
            return read::serialize_info(cx, st, out);
        }
        // R_InitReadItemDepth = -verbose: a depth below 0 prints the tags read
        let verbose = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(2))?;
        if verbose != 0 {
            return Err(cx.error("robigo: load(verbose = TRUE) is not implemented yet"));
        }
        cx.scope(|cx, s| {
            let o = s.out(cx);
            let ans = o.root();
            read::unserialize(cx, st, None, o)?;
            restore_to_env(cx, ans, args.root(1), out)
        })
    })();
    if !wasopen {
        connections::close_only(cx, n)?;
    }
    r
}

/// `RestoreToEnv(ans, aenv)`: the elements of the named list or pairlist
/// `ans` bound in `aenv`; their names.
// R: saveload.c:RestoreToEnv
fn restore_to_env(cx: &mut Cx, ans: Root<'_>, aenv: Root<'_>, out: Out<'_>) -> RResult<()> {
    /// `R_seemsOldStyleS4Object(obj)`: an object, not S4, whose class is
    /// one string with a "package" attribute.
    // R: objects.c:R_seemsOldStyleS4Object
    fn old_style_s4(x: Ref<'_>) -> bool {
        use robigo_rtbuiltins::attrib::get_value;
        if !x.is_object() || x.is_s4() {
            return false;
        }
        get_value(x, b"class").is_some_and(|k| k.len() == Some(1) && get_value(k, b"package").is_some())
    }
    let old_style = |cx: &mut Cx, obj: Root<'_>, name: &[u8]| -> RResult<()> {
        if old_style_s4(obj.bind(cx)) {
            let n = String::from_utf8_lossy(name).into_owned();
            cx.warning_call(None, &format!("'{n}' looks like a pre-2.4.0 S4 object: please recreate it"))?;
        }
        Ok(())
    };
    // allow ans to be a vector-style list
    if ans.bind(cx).sexptype() == VECSXP {
        return cx.scope(|cx, s| {
            let nm = robigo_rtbuiltins::attrib::get_value(ans.bind(cx), b"names").unwrap_or(cx.nil());
            let names = s.root(cx, nm);
            let len = ans.bind(cx).len().unwrap_or(0);
            if names.bind(cx).sexptype() != STRSXP || names.bind(cx).len() != Some(len) {
                return Err(cx.error("not a valid named list"));
            }
            for i in 0..len {
                cx.scope(|cx, s| {
                    let sym = robigo_rtbuiltins::envir::install_tr(cx, s, names, i)?;
                    let obj = s.root(cx, ans.bind(cx).as_vector::<Ref<'_>>().unwrap().get(i));
                    cx.define_var(sym, obj, aenv)?;
                    let name = names.bind(cx).as_vector::<Char<'_>>().unwrap().get(i).bytes().to_vec();
                    old_style(cx, obj, &name)
                })?;
            }
            out.set(cx, names.bind(cx));
            Ok(())
        });
    }
    if !matches!(ans.bind(cx).sexptype(), NILSXP | LISTSXP) {
        return Err(cx.error("loaded data is not in pair list form"));
    }
    let mut cells = Vec::new();
    let mut a = ans.bind(cx);
    while let Some(c) = a.as_cons() {
        let Some(t) = c.tag().as_sym() else {
            return Err(cx.error("robigo: load() of a pairlist with an untagged element is not implemented yet"));
        };
        cells.push(t.name(cx).bytes().to_vec());
        a = c.cdr();
    }
    cx.build(out, |cx| {
        let mut v = cx.alloc_vector::<Char<'_>>(cells.len())?;
        let mut a = ans.bind(cx);
        for i in 0..cells.len() {
            let c = a.as_cons().unwrap();
            v.set(i, c.tag().as_sym().unwrap().name(cx));
            a = c.cdr();
        }
        Ok(v.finish().into())
    })?;
    cx.scope(|cx, s| {
        let mut cell = ans;
        for name in &cells {
            let c = cell.bind(cx).as_cons().unwrap();
            let (tag, car, next) = (s.root(cx, c.tag()), s.root(cx, c.car()), s.root(cx, c.cdr()));
            cx.define_var(tag, car, aenv)?;
            old_style(cx, car, name)?;
            cell = next;
        }
        Ok(())
    })
}

/// `isValidStringF(x)`: a character vector whose first string is not
/// empty (NA's "NA" is not).
fn valid_string_f(x: Ref<'_>) -> bool {
    x.as_vector::<Char<'_>>().is_some_and(|v| !v.is_empty() && !v.get(0).bytes().is_empty())
}

/// `RC_fopen(fn, mode, TRUE)`'s file name: the native translation,
/// expanded; `None` for NA, which RC_fopen opens no file for.
// R: sysutils.c:RC_fopen
fn rc_name(cx: &mut Cx, x: Root<'_>) -> RResult<Option<std::path::PathBuf>> {
    if x.bind(cx).as_vector::<Char<'_>>().unwrap().get(0) == cx.na_string() {
        return Ok(None);
    }
    let name = {
        let c = x.bind(cx).as_vector::<Char<'_>>().unwrap().get(0);
        robigo_rtbuiltins::text::to_native_fp(robigo_rtbuiltins::coerce::text(c), cx.locale()).map(|b| b.into_owned())
    };
    let name = name.map_err(|m| cx.error(&m))?;
    let e = connections::r_expand_file_name(cx, &name)?;
    Ok(Some(std::path::PathBuf::from(<std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(&e))))
}

/// `defaultSaveVersion()`: `R_DEFAULT_SAVE_VERSION` when it is 2 or 3,
/// else 3, read once (GNU R's static).
// R: saveload.c:defaultSaveVersion
fn default_save_version() -> i32 {
    static DFLT: std::sync::OnceLock<i32> = std::sync::OnceLock::new();
    *DFLT.get_or_init(|| {
        std::env::var_os("R_DEFAULT_SAVE_VERSION")
            .map(|v| super::atoi(v.as_encoded_bytes()))
            .filter(|v| matches!(v, 2 | 3))
            .unwrap_or(3)
    })
}

/// `save.to.file(object, file, ascii, version)`: the magic number, then
/// the object serialized (ascii, or XDR).
// R: saveload.c:do_savefile, R_SaveToFileV
pub fn do_savefile(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "save.to.file", 4, true)?;
    if !valid_string_f(args.get(cx, 1)) {
        return Err(cx.error("'file' must be non-empty string"));
    }
    let ascii = args.get(cx, 2);
    if ascii.sexptype() != LGLSXP {
        return Err(cx.error("'ascii' must be logical"));
    }
    // INTEGER(ascii)[0] (NA is not 0)
    let ascii = match ascii.view() {
        robigo_rtcore::View::Lgl(v) if !v.is_empty() => v.get(0).0 != 0,
        _ => return Err(cx.error("robigo: save.to.file() with an empty 'ascii' is not implemented yet")),
    };
    let version = if args.get(cx, 3).sexptype() == NILSXP {
        default_save_version()
    } else {
        robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(3))?
    };
    if version == i32::MIN || version <= 0 {
        return Err(cx.error("invalid 'version' argument"));
    }
    let path = rc_name(cx, args.root(1))?;
    let Some(Ok(mut fp)) = path.map(std::fs::File::create) else {
        return Err(cx.error("unable to open 'file'"));
    };
    if version == 1 {
        return Err(cx.error("robigo: save.to.file(version = 1) is not implemented yet"));
    }
    let (magic, fmt) = match (ascii, version == 2) {
        (true, true) => (b"RDA2\n", Format::Ascii),
        (true, false) => (b"RDA3\n", Format::Ascii),
        (false, true) => (b"RDX2\n", Format::Xdr),
        (false, false) => (b"RDX3\n", Format::Xdr),
    };
    let bytes = write::serialize(cx, args.root(0), super::stream::OutStream::new(fmt, version), None)?;
    use std::io::Write;
    if fp.write_all(magic).and_then(|_| fp.write_all(&bytes)).is_err() {
        return Err(cx.error("write failed"));
    }
    out.set(cx, cx.nil());
    Ok(())
}

/// `load.from.file(file)`: the object a version 2 or 3 file holds.
// R: saveload.c:do_loadfile, R_LoadFromFile, R_ReadMagic
pub fn do_loadfile(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "load.from.file", 1, true)?;
    cx.scope(|cx, s| {
        let file =
            s.capture(cx, |cx, o| robigo_rtbuiltins::coerce::vector::coerce_vector(cx, args.root(0), STRSXP, o))?;
        if !valid_string_f(file.bind(cx)) {
            return Err(cx.error("bad file name"));
        }
        let path = rc_name(cx, file)?;
        let Some(Ok(data)) = path.map(std::fs::read) else {
            return Err(cx.error("unable to open 'file'"));
        };
        if data.len() < 5 {
            return Err(cx.error(if data.is_empty() {
                "restore file may be empty -- no data loaded"
            } else {
                "bad restore file magic number (file may be corrupted) -- no data loaded"
            }));
        }
        let fmt = match &data[..5] {
            b"RDA2\n" | b"RDA3\n" => Format::Ascii,
            b"RDB2\n" | b"RDB3\n" => Format::Binary,
            b"RDX2\n" | b"RDX3\n" => Format::Xdr,
            b"RDA1\n" | b"RDB1\n" | b"RDX1\n" => {
                return Err(cx.error("robigo: load.from.file() of a version 1 file is not implemented yet"));
            }
            m if m.starts_with(b"RD") => {
                return Err(cx.error("restore file may be from a newer version of R -- no data loaded"));
            }
            m => {
                // the digits of a version 1.0-1.3 file's number
                let d = |b: u8| (b as i32 - b'0' as i32) % 10;
                let num = d(m[3]) + 10 * d(m[2]) + 100 * d(m[1]) + 1000 * d(m[0]);
                if matches!(num, 1971 | 1972 | 1975 | 1976 | 1977) {
                    return Err(
                        cx.error("robigo: load.from.file() of a file from R before 1.4.0 is not implemented yet")
                    );
                }
                return Err(cx.error("bad restore file magic number (file may be corrupted) -- no data loaded"));
            }
        };
        let mut st = super::stream::InStream::memory(data[5..].to_vec());
        st.fmt = fmt;
        read::unserialize(cx, st, None, out)
    })
}
