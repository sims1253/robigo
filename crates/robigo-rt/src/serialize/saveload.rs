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
            let name = robigo_rtbuiltins::text::install_tr_char(cx, s, names, i)?;
            // VECTOR_ELT(values, i)
            let v = values.bind(cx);
            let Some(elt) = v.as_vector::<Ref<'_>>().filter(|_| matches!(v.sexptype(), VECSXP | EXPRSXP)) else {
                let t = robigo_rtbuiltins::util::type2char(v.sexptype());
                return Err(cx.error(&format!("VECTOR_ELT() can only be applied to a 'list', not a '{t}'")));
            };
            if i >= elt.len() {
                let len = elt.len();
                return Err(cx.error(&format!("attempt access index {i}/{len} in VECTOR_ELT")));
            }
            let e = s.root(cx, elt.get(i));
            let vo = s.out(cx);
            let val = vo.root();
            crate::eval::eval(cx, e, eenv, vo)?;
            let d = s.out(cx);
            let expr0 = d.root();
            cx.duplicate(expr, d)?;
            // SETCAR(CDR(expr0), val): SETCAR's "bad value" for a call with no
            // second element (CDR is R_NilValue)
            let Some(rest) = expr0.bind(cx).as_cons().map(|c| c.cdr()) else {
                return Err(cx.error("robigo: makeLazy() of an 'expr' that is not a call is not implemented yet"));
            };
            let Some(cell) = Some(rest).filter(|c| c.as_cons().is_some()) else {
                return Err(cx.error("bad value"));
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
        // R_InitReadItemDepth = R_ReadItemDepth = -verbose: while the depth
        // is at most 0 the reader prints the tags and list positions it reads
        let verbose = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(2))?;
        if verbose == i32::MIN {
            // (-NA_INTEGER overflows in GNU R's C)
            return Err(cx.error("robigo: load(verbose = NA) is not implemented yet"));
        }
        cx.scope(|cx, s| {
            let o = s.out(cx);
            let ans = o.root();
            read::unserialize_at(cx, st, None, -verbose, o)?;
            restore_to_env(cx, ans, args.root(1), out)
        })
    })();
    if !wasopen {
        connections::close_only(cx, n)?;
    }
    r
}

/// `saveToConn(list, con, ascii, version, environment, eval.promises)`:
/// the magic number, then the pairlist of `list`'s variables, found from
/// `environment` (promises forced with `eval.promises`), serialized to
/// `con` (opened "wb" for the call when it is closed).
// R: saveload.c:do_saveToConn
pub fn do_save_to_conn(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "saveToConn", 6, true)?;
    let list = args.root(0);
    if list.bind(cx).sexptype() != STRSXP {
        return Err(cx.error("first argument must be a character vector"));
    }
    let n = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(1))?;
    connections::with_conn(cx, n, |_| ())?;
    let ascii = robigo_rtbuiltins::util::as_bool2(cx, call, args.root(2))?;
    let version = if args.get(cx, 3).sexptype() == NILSXP {
        default_save_version()
    } else {
        robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(3))?
    };
    if version == i32::MIN || version <= 0 {
        return Err(cx.error("invalid 'version' argument"));
    }
    if version < 2 {
        return Err(cx.error(&format!("cannot save to connections in version {version} format")));
    }
    let source = args.root(4);
    if !matches!(source.bind(cx).sexptype(), NILSXP | ENVSXP) {
        return Err(cx.error("invalid 'environment' argument"));
    }
    let ep = robigo_rtbuiltins::util::as_logical_n(cx, args.get(cx, 5)).0;
    if ep == i32::MIN {
        return Err(cx.error("invalid 'eval.promises' argument"));
    }
    let wasopen = connections::flags(cx, n)?.isopen;
    if !wasopen && !connections::open_with_mode(cx, n, "wb")? {
        return Err(cx.error("cannot open the connection"));
    }
    // an error closes a connection this opened (the context's cend)
    let r = (|| -> RResult<()> {
        let f = connections::flags(cx, n)?;
        if !f.canwrite {
            return Err(cx.error("connection not open for writing"));
        }
        // (asBool2: ascii is never NA here, so never the asciihex format)
        let (magic, fmt) = if ascii {
            (b'A', Format::Ascii)
        } else {
            if f.text {
                return Err(cx.error("cannot save XDR format to a text-mode connection"));
            }
            (b'X', Format::Xdr)
        };
        // (a version above 3 writes its magic number, then R_Serialize refuses it)
        // magic[3] = (char)('0' + version): a NUL there ends the C string
        let magic = [b'R', b'D', magic, b'0'.wrapping_add(version as u8), b'\n'];
        let magic = &magic[..magic.iter().position(|&b| b == 0).unwrap_or(5)];
        if f.text {
            // Rconn_printf(con, "%s", magic): its result not looked at
            connections::write_ignoring_os_errors(cx, n, magic)?;
        } else {
            connections::write_all(cx, n, magic, "error writing to connection")?;
        }
        cx.scope(|cx, s| {
            let len = list.bind(cx).len().unwrap_or(0);
            let o = s.out(cx);
            let head = o.root();
            // allocList(len)
            cx.build(o, |cx| {
                let mut l = cx.nil();
                for _ in 0..len {
                    l = cx.cons(cx.nil(), l)?;
                }
                Ok(l)
            })?;
            let mut t = head;
            for j in 0..len {
                cx.scope(|cx, s| {
                    let tag = robigo_rtbuiltins::text::install_tr_char(cx, s, list, j)?;
                    let cell = s.root(cx, t.bind(cx));
                    cx.set_tag(cell, tag);
                    // SETCAR(t, R_findVar(TAG(t), source)), then the lookup
                    // again (an active binding runs twice)
                    let fo = s.out(cx);
                    let first = fo.root();
                    cx.find_var(tag, source, fo)?;
                    cx.set_car(cell, first);
                    let vo = s.out(cx);
                    let v = vo.root();
                    cx.find_var(tag, source, vo)?;
                    if v.bind(cx).sexp() == cx.unbound().sexp() {
                        // SAFETY: a rooted symbol.
                        let name = unsafe { crate::eval::sym_name(cx, tag.bind(cx).sexp()) };
                        return Err(cx.error(&format!("object '{name}' not found")));
                    }
                    if ep != 0 && v.bind(cx).sexptype() == PROMSXP {
                        let fo = s.out(cx);
                        let fv = fo.root();
                        crate::eval::eval(cx, v, source, fo)?;
                        cx.set_car(cell, fv);
                    } else {
                        cx.set_car(cell, v);
                    }
                    Ok(())
                })?;
                let next = t.bind(cx).as_cons().unwrap().cdr();
                t = s.root(cx, next);
            }
            super::serialize_to_conn(cx, n, head, fmt, version, None)
        })
    })();
    if !wasopen {
        connections::close_only(cx, n)?;
    }
    r?;
    out.set(cx, cx.nil());
    Ok(())
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
    let old_style = |cx: &mut Cx, name: &[u8]| -> RResult<()> {
        let n = String::from_utf8_lossy(name).into_owned();
        cx.warning_call(None, &format!("'{n}' looks like a pre-2.4.0 S4 object: please recreate it"))
    };
    // allow ans to be a vector-style list
    if ans.bind(cx).sexptype() == VECSXP {
        return cx.scope(|cx, s| {
            // getAttrib(ans, R_NamesSymbol): a 1-d array's dimnames[[1]] too
            let nm = robigo_rtbuiltins::attrib::names(ans.bind(cx)).unwrap_or(cx.nil());
            let names = s.root(cx, nm);
            let len = ans.bind(cx).len().unwrap_or(0);
            if names.bind(cx).sexptype() != STRSXP || names.bind(cx).len() != Some(len) {
                return Err(cx.error("not a valid named list"));
            }
            for i in 0..len {
                cx.scope(|cx, s| {
                    let sym = robigo_rtbuiltins::text::install_tr_char(cx, s, names, i)?;
                    let obj = s.root(cx, ans.bind(cx).as_vector::<Ref<'_>>().unwrap().get(i));
                    cx.define_var(sym, obj, aenv)?;
                    if !old_style_s4(obj.bind(cx)) {
                        return Ok(());
                    }
                    // CHAR(STRING_ELT(names, i))
                    let name = names.bind(cx).as_vector::<Char<'_>>().unwrap().get(i).bytes().to_vec();
                    old_style(cx, &name)
                })?;
            }
            out.set(cx, names.bind(cx));
            Ok(())
        });
    }
    if !matches!(ans.bind(cx).sexptype(), NILSXP | LISTSXP) {
        return Err(cx.error("loaded data is not in pair list form"));
    }
    let mut cells = 0;
    let mut a = ans.bind(cx);
    while let Some(c) = a.as_cons() {
        if c.tag().as_sym().is_none() {
            return Err(cx.error("robigo: load() of a pairlist with an untagged element is not implemented yet"));
        }
        cells += 1;
        a = c.cdr();
    }
    cx.build(out, |cx| {
        let mut v = cx.alloc_vector::<Char<'_>>(cells)?;
        let mut a = ans.bind(cx);
        for i in 0..cells {
            let c = a.as_cons().unwrap();
            v.set(i, c.tag().as_sym().unwrap().name(cx));
            a = c.cdr();
        }
        Ok(v.finish().into())
    })?;
    cx.scope(|cx, s| {
        let mut cell = ans;
        for _ in 0..cells {
            let c = cell.bind(cx).as_cons().unwrap();
            let (tag, car, next) = (s.root(cx, c.tag()), s.root(cx, c.car()), s.root(cx, c.cdr()));
            cx.define_var(tag, car, aenv)?;
            if old_style_s4(car.bind(cx)) {
                let name = tag.bind(cx).as_sym().unwrap().name(cx).bytes().to_vec();
                old_style(cx, &name)?;
            }
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
    // (refused before the open, which would empty the file)
    if version == 1 {
        return Err(cx.error("robigo: save.to.file(version = 1) is not implemented yet"));
    }
    let path = rc_name(cx, args.root(1))?;
    let Some(Ok(mut fp)) = path.map(std::fs::File::create) else {
        return Err(cx.error("unable to open 'file'"));
    };
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
        let Some(Ok(mut data)) = path.map(std::fs::read) else {
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
        data.drain(..5);
        let mut st = super::stream::InStream::memory(data);
        st.fmt = fmt;
        read::unserialize(cx, st, None, out)
    })
}
