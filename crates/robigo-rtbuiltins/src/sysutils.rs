//! sysutils.c's `interactive()`, `.Internal(Sys.getenv(x, unset))`,
//! `.Internal(Sys.setenv(nm, values))` and `.Internal(Sys.unsetenv(nm))`,
//! and platform.c's `.Internal(l10n_info())`, `Sys.getlocale` and
//! `Sys.setlocale`.

use crate::attrib::{Attr, Attrs};
use robigo_rtcore::RSym;
use robigo_rtcore::clib::category as lc;
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

/// `translateChar(STRING_ELT(x, i))` of a character vector that is not a
/// package ALTREP one (checked by the caller), or the message of GNU R's
/// error, raised by the caller once it holds no environment writer.
// R: sysutils.c:translateChar
fn translated(cx: &Cx, x: Root<'_>, i: usize) -> Result<Vec<u8>, String> {
    let ch = x.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
    crate::text::to_native(crate::coerce::text(ch), cx.locale()).map(|b| b.into_owned()).map_err(|e| e.to_string())
}

/// The process environment cannot be written now: this isolate holds no
/// grant (`clib::EnvWrites`), or `clib::env_write_allowed` refused (a glibc
/// older than `clib::MIN_GLIBC` and several isolates).
fn refused(cx: &mut Cx, call: Root<'_>, f: &str, why: robigo_rtcore::clib::EnvWriteRefused) -> robigo_rtcore::RError {
    use robigo_rtcore::clib::EnvWriteRefused::*;
    let what = match why {
        NotGranted => format!("{f} in an isolate that was not given the process environment"),
        SeveralIsolates => {
            let (maj, min) = robigo_rtcore::clib::MIN_GLIBC;
            format!("{f} with several isolates on glibc older than {maj}.{min}")
        }
    };
    crate::util::not_yet(cx, Some(call), &what)
}

/// `.Internal(Sys.setenv(nm, values))`: set each variable `nm[i]` to
/// `values[i]` in the process's environment (`setenv`, so every isolate
/// sees it; see `clib::EnvWriter::set`), each TRUE if it was set.
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
    // one write permission for the whole call, taken before the first
    // element: a refusal changes nothing
    let w = match n {
        0 => None,
        _ => match robigo_rtcore::clib::env_writer(cx) {
            Ok(w) => Some(w),
            Err(why) => return Err(refused(cx, call, "Sys.setenv()", why)),
        },
    };
    // each set as it is translated, as GNU R's loop does (an error leaves
    // the earlier ones set). Nothing here starts an isolate or reads the
    // environment through clib: translation is robigo-text's own code, and
    // its error is raised after the writer is dropped.
    let mut ok = Vec::with_capacity(n);
    for i in 0..n {
        let pair = translated(cx, nm, i).and_then(|name| Ok((name, translated(cx, val, i)?)));
        match (pair, &w) {
            (Ok((name, value)), Some(w)) => ok.push(Lgl(w.set(&name, &value) as i32)),
            (Err(m), _) => {
                drop(w);
                return Err(cx.error(&m));
            }
            (Ok(_), None) => unreachable!("robigo: an element of an empty vector"),
        }
    }
    drop(w);
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
    if n == 0 {
        return cx.build(out, |cx| Ok(cx.alloc_vector::<Lgl>(0)?.finish().into()));
    }
    // one write permission for the whole call (both passes: the second
    // reads an environment no one else changes meanwhile); as for setenv
    let w = match robigo_rtcore::clib::env_writer(cx) {
        Ok(w) => w,
        Err(why) => return Err(refused(cx, call, "Sys.unsetenv()", why)),
    };
    for i in 0..n {
        match translated(cx, nm, i) {
            Ok(name) => w.unset(&name),
            Err(m) => {
                drop(w);
                return Err(cx.error(&m));
            }
        }
    }
    let mut ok = Vec::with_capacity(n);
    for i in 0..n {
        // (translated above, so it cannot fail now)
        let name = translated(cx, nm, i).unwrap_or_default();
        ok.push(Lgl(!w.has(&name) as i32));
    }
    drop(w);
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

/// One `iconv` conversion of `do_iconv`, opened on first use: the
/// descriptor and whether its input is UTF-8 (for `sub = "Unicode"` and
/// `"c99"`).
struct Conv {
    cd: robigo_rtcore::clib::Iconv,
    from_utf8: bool,
}

/// `Riconv_open(to, from)` for `do_iconv` (Linux: `iconv_open`, "utf8"
/// as UTF-8). The C library reads "" as the process's charset; robigo-rt's
/// process stays in "C" and the locale is the isolate's, so "" is the
/// isolate's charset here, as GNU R's is its process's.
// R: sysutils.c:Riconv_open
fn riconv_open(cx: &mut Cx, to: &[u8], from: &[u8]) -> RResult<Option<robigo_rtcore::clib::Iconv>> {
    let native: &[u8] = if cx.locale().utf8 { b"UTF-8" } else { b"ANSI_X3.4-1968" };
    let fix = |e: &[u8]| -> Vec<u8> {
        if e.is_empty() {
            native.to_vec()
        } else if e.eq_ignore_ascii_case(b"utf8") {
            b"UTF-8".to_vec()
        } else {
            e.to_vec()
        }
    };
    use robigo_rtcore::clib::{Iconv, IconvOpenError};
    match Iconv::open(&fix(to), &fix(from), cx.locale().utf8) {
        Ok(cd) => Ok(Some(cd)),
        Err(IconvOpenError::Unsupported) => Ok(None),
        Err(IconvOpenError::NoLocale) => {
            Err(cx.error("robigo: the C library has no locale object for iconv()'s LC_CTYPE"))
        }
    }
}

/// `.Internal(iconv(x, from, to, sub, mark, toRaw))`: each string (or raw
/// vector of a list) converted by the C library's `iconv`; what it cannot
/// convert is NA (NULL with `toRaw`) unless `sub` says what to write
/// instead: `"byte"` (`<xx>` per byte), `"Unicode"` (`<U+XXXX>`) and `"c99"`
/// (`\uxxxx`, `\Uxxxxxxxx`) for UTF-8 input, or any string. `x = NULL`
/// lists the conversions: NULL on Linux (glibc has no `iconvlist`).
// R: sysutils.c:do_iconv
pub fn do_iconv(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("iconv", true).check_arity(cx, call, args)?;
    let x = args.root(0);
    if x.bind(cx).sexptype() == NILSXP {
        // (HAVE_ICONVLIST is libiconv's)
        out.set(cx, cx.nil());
        return Ok(());
    }
    let one_string = |cx: &Cx, i: usize| -> Option<Option<Vec<u8>>> {
        let v = args.get(cx, i);
        if v.sexptype() != STRSXP || v.len() != Some(1) {
            return None;
        }
        let c = v.as_vector::<Char<'_>>().unwrap().get(0);
        Some((c != cx.na_string()).then(|| c.bytes().to_vec()))
    };
    for (i, name) in [(1, "from"), (2, "to"), (3, "sub")] {
        if args.get(cx, i).is_package_altrep() {
            return Err(crate::util::not_yet(cx, Some(call), "iconv() of a package ALTREP vector"));
        }
        if one_string(cx, i).is_none() {
            return Err(cx.error(&format!("invalid '{name}' argument")));
        }
    }
    // (ASCII: CHAR as is; NA reads as "NA")
    let from = one_string(cx, 1).unwrap().unwrap_or_else(|| b"NA".to_vec());
    let to = one_string(cx, 2).unwrap().unwrap_or_else(|| b"NA".to_vec());
    let mut sub: Option<Vec<u8>> = match one_string(cx, 3).unwrap() {
        None => None,
        Some(_) => Some(crate::text::native_elt(cx, args.root(3), 0)?),
    };
    let mark = crate::util::as_logical(cx, args.get(cx, 4));
    if mark == i32::MIN {
        return Err(cx.error("invalid 'mark' argument"));
    }
    let to_raw = crate::util::as_logical(cx, args.get(cx, 5));
    if to_raw == i32::MIN {
        return Err(cx.error("invalid 'toRaw' argument"));
    }
    let (mark, to_raw) = (mark != 0, to_raw != 0);
    // some iconv's allow "UTF8", but GNU libiconv does not
    let from = if from.eq_ignore_ascii_case(b"UTF8") { b"UTF-8".to_vec() } else { from };
    let mut to = if to.eq_ignore_ascii_case(b"UTF8") { b"UTF-8".to_vec() } else { to };
    let utf8_locale = cx.locale().utf8;
    let is_utf8 = to == b"UTF-8" || (to.is_empty() && utf8_locale);
    let is_latin1 = [&b"latin1"[..], b"ISO_8859-1", b"CP1252"].iter().any(|e| to.eq_ignore_ascii_case(e));
    let xv = x.bind(cx);
    let raw_list = xv.sexptype() == VECSXP;
    if !raw_list && xv.sexptype() != STRSXP {
        return Err(cx.error("'x' must be a character vector"));
    }
    if xv.is_package_altrep() {
        return Err(crate::util::not_yet(cx, Some(call), "iconv() of a package ALTREP vector"));
    }
    let n = xv.len().unwrap();
    // each element's result: Some(bytes) converted, None failed or NA/NULL
    // (the input's own value kept: see below)
    let ienc = if !mark {
        Encoding::Native
    } else if is_latin1 {
        Encoding::Latin1
    } else if is_utf8 {
        Encoding::Utf8
    } else {
        Encoding::Native
    };
    // cbuff's size (R_AllocStringBuffer(0, &cbuff))
    let mut bufsize = string_buffer_size(0, 0);
    let mut results: Vec<Option<Vec<u8>>> = Vec::with_capacity(n);
    let (mut arg_obj, mut latin1_obj, mut utf8_obj): (Option<Conv>, Option<Conv>, Option<Conv>) = (None, None, None);
    for i in 0..n {
        // the input bytes and its mark, or None for NA / NULL
        let elt: Option<(Vec<u8>, Encoding)> = {
            let xv = x.bind(cx);
            if raw_list {
                let si = xv.as_vector::<Ref<'_>>().unwrap().get(i);
                match si.sexptype() {
                    NILSXP => None,
                    RAWSXP if si.is_package_altrep() => {
                        return Err(crate::util::not_yet(cx, Some(call), "iconv() of a package ALTREP vector"));
                    }
                    RAWSXP => Some((si.as_vector::<u8>().unwrap().as_slice().unwrap().to_vec(), Encoding::Native)),
                    _ => {
                        return Err(cx.error("'x' must be a character vector or a list of NULL or raw vectors"));
                    }
                }
            } else {
                let c = xv.as_vector::<Char<'_>>().unwrap().get(i);
                (c != cx.na_string()).then(|| (c.bytes().to_vec(), c.enc()))
            }
        };
        let Some((inp, enc)) = elt else {
            results.push(None);
            continue;
        };
        // With 'from = ""', encoding flags are used in preference of native
        // encoding
        let unsupported = |cx: &mut Cx, f: &[u8], t: &[u8]| {
            let m = format!(
                "unsupported conversion from '{}' to '{}'",
                String::from_utf8_lossy(f),
                String::from_utf8_lossy(t)
            );
            cx.error(&m)
        };
        let obj: &mut Conv = if !raw_list && enc == Encoding::Utf8 && from.is_empty() {
            if utf8_obj.is_none() {
                let mut cd = riconv_open(cx, &to, b"UTF-8")?;
                if cd.is_none() && to == b"ASCII//TRANSLIT" {
                    to = b"ASCII".to_vec();
                    cd = riconv_open(cx, &to, b"UTF-8")?;
                    if sub.is_none() {
                        sub = Some(b"c99".to_vec());
                    }
                }
                let Some(cd) = cd else { return Err(unsupported(cx, b"UTF-8", &to)) };
                utf8_obj = Some(Conv { cd, from_utf8: true });
            }
            utf8_obj.as_mut().unwrap()
        } else if !raw_list && enc == Encoding::Latin1 && from.is_empty() {
            if latin1_obj.is_none() {
                let mut cd = riconv_open(cx, &to, b"latin1")?;
                if cd.is_none() && to == b"ASCII//TRANSLIT" {
                    to = b"ASCII".to_vec();
                    cd = riconv_open(cx, &to, b"latin1")?;
                    if sub.is_none() {
                        sub = Some(b"?".to_vec());
                    }
                }
                let Some(cd) = cd else { return Err(unsupported(cx, b"latin1", &to)) };
                latin1_obj = Some(Conv { cd, from_utf8: false });
            }
            latin1_obj.as_mut().unwrap()
        } else {
            if arg_obj.is_none() {
                let mut cd = riconv_open(cx, &to, &from)?;
                if cd.is_none() && to == b"ASCII//TRANSLIT" {
                    to = b"ASCII".to_vec();
                    cd = riconv_open(cx, &to, &from)?;
                    if sub.is_none() {
                        sub = Some(b"?".to_vec());
                    }
                }
                let Some(cd) = cd else { return Err(unsupported(cx, &from, &to)) };
                arg_obj = Some(Conv { cd, from_utf8: from == b"UTF-8" || (from.is_empty() && utf8_locale) });
            }
            arg_obj.as_mut().unwrap()
        };
        let r = match convert_one(obj, &inp, sub.as_deref(), &from, &mut bufsize) {
            Ok(r) => r,
            Err(what) => return Err(crate::util::not_yet(cx, Some(call), what)),
        };
        // mkCharLenCE as each element is done: its error for an embedded
        // NUL comes before the next element is looked at
        if !to_raw && let Some(b) = &r {
            cx.scope(|cx, s| s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.mk_char(b, ienc)?.into()))).map(|_| ()))?;
        }
        results.push(r);
    }
    cx.build(out, |cx| {
        let xv = x.bind(cx);
        // SHALLOW_DUPLICATE_ATTRIB(ans, x) (or duplicate(x)): the attributes
        let mut a = Attrs::new();
        a.copy_all(xv);
        if to_raw {
            let mut ans = cx.alloc_vector::<Ref<'_>>(n)?;
            for (i, r) in results.iter().enumerate() {
                let v = match r {
                    Some(b) => cx.alloc_copy(b)?.finish().into(),
                    // (a raw list's duplicate keeps the input; a character
                    // vector's new list has NULL)
                    None if raw_list => xv.as_vector::<Ref<'_>>().unwrap().get(i),
                    None => cx.nil(),
                };
                ans.set(i, v);
            }
            a.apply(cx, &mut ans)?;
            Ok(ans.finish().into())
        } else {
            let mut ans = cx.alloc_vector::<Char<'_>>(n)?;
            for (i, r) in results.iter().enumerate() {
                ans.set(
                    i,
                    match r {
                        Some(b) => cx.mk_char(b, ienc)?,
                        None => cx.na_string(),
                    },
                );
            }
            a.apply(cx, &mut ans)?;
            Ok(ans.finish().into())
        }
    })
}

/// `R_AllocStringBuffer(blen, &cbuff)`'s new size for a buffer of `cur`
/// bytes (`MAXELTSIZE`, 8192, its default): unchanged if `blen` fits,
/// else `blen + 1` rounded up to a multiple of 8192.
// R: memory.c:R_AllocStringBuffer
fn string_buffer_size(cur: usize, blen: usize) -> usize {
    const BSIZE: usize = 8192;
    if blen < cur {
        return cur;
    }
    let blen1 = blen + 1;
    let b = (blen1 / BSIZE) * BSIZE;
    if b < blen1 { b + BSIZE } else { b }
}

/// One element through `obj`, do_iconv's loop from `top_of_loop` with its
/// string buffer of `*bufsize` bytes (kept from element to element, as
/// `cbuff` is): a full buffer is doubled and the element converted again
/// from the start, after the reset of the descriptor's state (whose shift
/// sequence then starts the output, as in GNU R). The converted bytes, None
/// when the conversion stopped with no `sub`, or the not-implemented case
/// GNU R loops on for ever.
// R: sysutils.c:do_iconv
fn convert_one(
    obj: &mut Conv,
    inp: &[u8],
    sub: Option<&[u8]>,
    from: &[u8],
    bufsize: &mut usize,
) -> Result<Option<Vec<u8>>, &'static str> {
    use robigo_rtcore::clib::IconvStop;
    'top_of_loop: loop {
        let mut buf = vec![0u8; *bufsize];
        let mut pos = 0;
        let (mut o, mut outb) = (0usize, *bufsize - 1);
        // First initialize output
        let (_, w, _) = obj.cd.iconv(None, &mut buf[o..o + outb]);
        o += w;
        outb -= w;
        // next_char:
        loop {
            // Then convert input
            let (k, w, stop) = obj.cd.iconv(Some(&inp[pos..]), &mut buf[o..o + outb]);
            pos += k;
            o += w;
            outb -= w;
            let grow = |bufsize: &mut usize| *bufsize = string_buffer_size(*bufsize, 2 * *bufsize);
            match (stop, sub) {
                (None, _) => {
                    buf.truncate(o);
                    // (res != -1 && inb == 0)
                    return Ok((pos == inp.len()).then_some(buf));
                }
                (Some(IconvStop::TooBig), _) => {
                    grow(bufsize);
                    continue 'top_of_loop;
                }
                (Some(_), None) => return Ok(None),
                (Some(_), Some(sub)) => {
                    // (EILSEQ or EINVAL: it seems this gets thrown for
                    // non-convertible input too)
                    let left = inp.len() - pos;
                    if obj.from_utf8 && (sub == b"Unicode" || sub == b"c99") {
                        if outb < if sub == b"Unicode" { 13 } else { 11 } {
                            grow(bufsize);
                            continue 'top_of_loop;
                        }
                        let (clen, wc) = robigo_text::utf8toucs(inp, pos);
                        if !(clen > 0 && left >= clen as usize) {
                            // (GNU R goes to next_char without consuming:
                            // it loops for ever on what is not UTF-8)
                            return Err("iconv() with sub = \"Unicode\" or \"c99\" of input that is not UTF-8");
                        }
                        let ucs = if robigo_text::is_high_surrogate(wc) {
                            robigo_text::utf8toucs32(wc, inp, pos)
                        } else {
                            wc as u32
                        };
                        pos += clen as usize;
                        let t = match (sub == b"Unicode", ucs < 65536) {
                            (true, _) => format!("<U+{ucs:04X}>"),
                            (false, true) => format!("\\u{ucs:04x}"),
                            (false, false) => format!("\\U{ucs:08x}"),
                        };
                        buf[o..o + t.len()].copy_from_slice(t.as_bytes());
                        o += t.len();
                        outb -= t.len();
                        continue;
                    }
                    let pick = |p: &[u8], n: usize| from.len() >= n && from[..n].eq_ignore_ascii_case(p);
                    let unit = if pick(b"UTF-16", 6) || pick(b"UCS-2", 5) {
                        2
                    } else if pick(b"UTF-32", 6) || pick(b"UCS-4", 5) {
                        4
                    } else {
                        1
                    };
                    let mut u = 0;
                    while u < unit && pos < inp.len() {
                        let t: Vec<u8> = if sub == b"byte" {
                            if outb < 5 {
                                grow(bufsize);
                                continue 'top_of_loop;
                            }
                            format!("<{:02x}>", inp[pos]).into_bytes()
                        } else {
                            if outb < sub.len() {
                                grow(bufsize);
                                continue 'top_of_loop;
                            }
                            sub.to_vec()
                        };
                        buf[o..o + t.len()].copy_from_slice(&t);
                        o += t.len();
                        outb -= t.len();
                        pos += 1;
                        u += 1;
                    }
                }
            }
        }
    }
}

/// R's locale category numbers (`Sys.getlocale`'s `category` index):
/// 1 `LC_ALL`, 2 `LC_COLLATE`, ... 9 `LC_MEASUREMENT`.
fn r_category(cat: i32) -> Option<lc::c_int> {
    Some(match cat {
        1 => lc::LC_ALL,
        2 => lc::LC_COLLATE,
        3 => lc::LC_CTYPE,
        4 => lc::LC_MONETARY,
        5 => lc::LC_NUMERIC,
        6 => lc::LC_TIME,
        7 => lc::LC_MESSAGES,
        8 => lc::LC_PAPER,
        9 => lc::LC_MEASUREMENT,
        _ => return None,
    })
}

/// `asInteger(category)`, an error for NA or a negative number (both
/// builtins' first check).
fn category_number(cx: &mut Cx, args: Args<'_>) -> RResult<i32> {
    let cat = crate::coerce::vector::as_integer_warn(cx, args.root(0))?;
    if cat == i32::MIN || cat < 0 {
        return Err(cx.error("invalid 'category' argument"));
    }
    Ok(cat)
}

/// `.Internal(Sys.getlocale(category))`: the category's locale name
/// (`LC_ALL`'s composite), "" for a number that names none.
// R: platform.c:do_getlocale
pub fn do_getlocale(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("Sys.getlocale", true).check_arity(cx, call, args)?;
    let cat = category_number(cx, args)?;
    let p = r_category(cat).map(|c| cx.locale_name(c)).unwrap_or_default();
    cx.build(out, |cx| cx.mk_string(&p, Encoding::Native))
}

/// A C-like locale: what robigo-rt does in every category but
/// `LC_COLLATE` and `LC_CTYPE` (the C library's "C" and "C.UTF-8").
fn c_like(name: &[u8]) -> bool {
    matches!(name, b"C" | b"POSIX" | b"C.UTF-8" | b"C.utf8")
}

/// `.Internal(Sys.setlocale(category, locale))`: set the category's
/// locale; the new name ("" with a warning when the C library has no such
/// locale). robigo-rt models what the locale changes for R only so far:
/// `LC_CTYPE` only between names of the same C locale, `LC_COLLATE` not
/// at all, the other categories between the C-like locales; any other
/// change is refused.
// R: platform.c:do_setlocale
pub fn do_setlocale(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("Sys.setlocale", true).check_arity(cx, call, args)?;
    let cat = category_number(cx, args)?;
    let l = match args.get(cx, 1).as_vector::<Char<'_>>() {
        Some(v) if v.len() == 1 => v.get(0).bytes().to_vec(),
        _ => return Err(cx.error("invalid 'locale' argument")),
    };
    let Some(c) = r_category(cat) else {
        return Err(cx.error("invalid 'category' argument"));
    };
    let old = cx.locale_names();
    let mut new = old.clone();
    let p = if c == lc::LC_ALL {
        // assume we can set LC_CTYPE iff we can set the rest
        new.set(lc::LC_CTYPE, &l).map(|_| {
            for c in [lc::LC_COLLATE, lc::LC_MONETARY, lc::LC_TIME] {
                new.set(c, &l);
            }
            new.get(lc::LC_ALL)
        })
    } else {
        if c == lc::LC_NUMERIC && l != b"C" {
            // do not complain about C locale - that's the only reliable way
            // to restore sanity
            cx.warning("setting 'LC_NUMERIC' may cause R to function strangely")?;
        }
        new.set(c, &l)
    };
    // what robigo-rt models of the change (before any of it is made)
    for (c, n) in [
        (lc::LC_CTYPE, "LC_CTYPE"),
        (lc::LC_COLLATE, "LC_COLLATE"),
        (lc::LC_MONETARY, "LC_MONETARY"),
        (lc::LC_NUMERIC, "LC_NUMERIC"),
        (lc::LC_TIME, "LC_TIME"),
        (lc::LC_MESSAGES, "LC_MESSAGES"),
        (lc::LC_PAPER, "LC_PAPER"),
        (lc::LC_MEASUREMENT, "LC_MEASUREMENT"),
    ] {
        let (a, b) = (old.get(c), new.get(c));
        let modelled = a == b
            || match c {
                lc::LC_COLLATE => false,
                // the character classes and case mapping are C.UTF-8's (or C's):
                // only another name of the same locale
                lc::LC_CTYPE => {
                    let c_utf8 = |n: &[u8]| matches!(n, b"C.UTF-8" | b"C.utf8");
                    c_utf8(&a) && c_utf8(&b)
                }
                _ => c_like(&a) && c_like(&b),
            };
        if !modelled {
            let (a, b) = (String::from_utf8_lossy(&a), String::from_utf8_lossy(&b));
            return Err(
                cx.error(&format!("robigo: Sys.setlocale() changing {n} from '{a}' to '{b}' is not implemented yet"))
            );
        }
    }
    cx.set_locale_names(new);
    let Some(p) = p else {
        cx.build(out, |cx| cx.mk_string(b"", Encoding::Native))?;
        let l = String::from_utf8_lossy(&l).into_owned();
        return cx.warning(&format!("OS reports request to set locale to \"{l}\" cannot be honored"));
    };
    // (R_check_locale: the codeset is unchanged)
    cx.build(out, |cx| cx.mk_string(&p, Encoding::Native))
}
