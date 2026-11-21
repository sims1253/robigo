//! connections.c's text readers (L4): `isOpen`, `isIncomplete`, the
//! pushback (`pushBack`, `pushBackLength`, `clearPushBack`) and
//! `readLines`. They read through [`conn_fgetc`] (`Rconn_fgetc`), as GNU
//! R's do.

use super::{
    Conn, NA_INTEGER, R_EOF, close_conn, conn_fgetc, conn_number, flags, open_with_mode, our_fgetc, rconn_seek,
    with_conn,
};
use robigo_heap::types::STRSXP;
use robigo_rtbuiltins::coerce::vector::as_integer_warn;
use robigo_rtbuiltins::util::as_logical;
use robigo_rtcore::View;
use robigo_rtcore::{Args, Char, Cx, Encoding, Lgl, Out, RResult, Root};
use std::ffi::c_int;

/// `isOpen(con, rw)`: whether the connection is open, and for `rw` 1 or 2
/// whether it can read or write.
// R: connections.c:do_isopen
pub fn do_isopen(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "isOpen", 2, true)?;
    let n = as_integer_warn(cx, args.root(0))?;
    let (isopen, canread, canwrite) = with_conn(cx, n, |c| (c.c.isopen != 0, c.c.canread, c.c.canwrite))?;
    let rw = as_integer_warn(cx, args.root(1))?;
    let mut res = isopen as c_int;
    match rw {
        0 => {}
        1 => res &= canread,
        2 => res &= canwrite,
        _ => return Err(cx.error("unknown 'rw' value")),
    }
    cx.build_scalar(out, Lgl(res))
}

/// `isIncomplete(con)`.
// R: connections.c:do_isincomplete
pub fn do_isincomplete(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "isIncomplete", 1, true)?;
    let n = conn_number(cx, args.root(0))?;
    let inc = with_conn(cx, n, |c| c.incomplete())?;
    cx.build_scalar(out, Lgl(inc as i32))
}

/// `pushBack(data, connection, newLine, type)`: the strings on top of the
/// connection's pushback, `data[1]` read first; `type` 1 translates them
/// to the native encoding, 3 to UTF-8, anything else takes their bytes.
// R: connections.c:do_pushback
pub fn do_pushback(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "pushBack", 4, true)?;
    if args.get(cx, 0).sexptype() != STRSXP {
        return Err(cx.error("invalid 'data' argument"));
    }
    let n = as_integer_warn(cx, args.root(1))?;
    with_conn(cx, n, |_| ())?;
    let new_line = as_logical(cx, args.get(cx, 2));
    if new_line == NA_INTEGER {
        return Err(cx.error("invalid 'newLine' argument"));
    }
    let ty = as_integer_warn(cx, args.root(3))?;
    let f = flags(cx, n)?;
    if !f.canread && !f.isopen {
        return Err(cx.error("can only push back on open readable connections"));
    }
    if !f.text {
        return Err(cx.error("can only push back on text-mode connections"));
    }
    // the lines, last string first (the top of the pushback is the last);
    // a translation's error pushes none (GNU R's count is set after)
    let mut lines = Vec::new();
    {
        let Some(v) = args.get(cx, 0).as_vector::<Char<'_>>() else {
            return Err(robigo_rtbuiltins::util::not_yet(
                cx,
                Some(call),
                "pushBack() of a package ALTREP character vector",
            ));
        };
        let loc = cx.locale();
        for i in (0..v.len()).rev() {
            let c = v.get(i);
            let b = if c == cx.na_string() || (ty != 1 && ty != 3) {
                Ok(c.bytes().to_vec())
            } else {
                let s = robigo_rtbuiltins::coerce::text(c);
                let r = if ty == 1 {
                    robigo_rtbuiltins::text::to_native(s, loc)
                } else {
                    robigo_rtbuiltins::text::to_utf8(s, loc)
                };
                r.map(|b| b.into_owned())
            };
            match b {
                Ok(b) => lines.push(b),
                Err(_) => {
                    lines.clear();
                    break;
                }
            }
        }
        if lines.len() != v.len() {
            return Err(cx.error("translating strings with \"bytes\" encoding is not allowed"));
        }
    }
    if !lines.is_empty() {
        let r = with_conn(cx, n, |c| lines.iter().try_for_each(|l| c.push_back_line(l, new_line != 0)))?;
        r.map_err(|m| cx.error(&m))?;
    }
    out.set(cx, cx.nil());
    Ok(())
}

/// `pushBackLength(connection)`.
// R: connections.c:do_pushbacklength
pub fn do_pushbacklength(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "pushBackLength", 1, true)?;
    let n = as_integer_warn(cx, args.root(0))?;
    let len = with_conn(cx, n, |c| c.push_back_len())?;
    cx.build_scalar(out, len as i32)
}

/// `clearPushBack(connection)`.
// R: connections.c:do_clearpushback
pub fn do_clearpushback(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "clearPushBack", 1, true)?;
    let n = as_integer_warn(cx, args.root(0))?;
    with_conn(cx, n, |c| {
        if c.push_back_len() > 0 {
            c.clear_push_back();
        }
    })?;
    out.set(cx, cx.nil());
    Ok(())
}

/// One line of `readLines`' loop: the bytes up to a newline or the end
/// (NULs dropped when `skip_nul`), and the character that ended it.
// R: connections.c:do_readLines (the inner loop)
fn read_line(c: &mut Conn, skip_nul: bool, ours: Option<usize>) -> Result<(Vec<u8>, c_int), String> {
    let mut buf = Vec::new();
    loop {
        let ch = conn_fgetc(c, ours)?;
        if ch == R_EOF {
            return Ok((buf, ch));
        }
        if skip_nul && ch == 0 {
            continue;
        }
        if ch == b'\n' as c_int {
            return Ok((buf, ch));
        }
        buf.push(ch as u8);
    }
}

/// `strlen(buf)`.
fn strlen(b: &[u8]) -> usize {
    b.iter().position(|&x| x == 0).unwrap_or(b.len())
}

/// `readLines(con, n, ok, warn, encoding, skipNul)`.
// R: connections.c:do_readLines
pub fn do_readlines(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "readLines", 6, true)?;
    let ncon = conn_number(cx, args.root(0))?;
    with_conn(cx, ncon, |_| ())?;
    let n = match robigo_rtbuiltins::builtin::as_vec_size_signed(cx, args, 1)? {
        Err(m) => return Err(cx.error(m)),
        Ok(-999) => return Err(cx.error("invalid 'n' argument")),
        Ok(n) => n,
    };
    let ok = as_logical(cx, args.get(cx, 2));
    if ok == NA_INTEGER {
        return Err(cx.error("invalid 'ok' argument"));
    }
    let warn = as_logical(cx, args.get(cx, 3));
    if warn == NA_INTEGER {
        return Err(cx.error("invalid 'warn' argument"));
    }
    let encoding = match args.get(cx, 4).as_vector::<Char<'_>>() {
        Some(v) if v.len() == 1 => v.get(0).bytes().to_vec(),
        _ => return Err(cx.error("invalid 'encoding' value")),
    };
    let skip_nul = as_logical(cx, args.get(cx, 5));
    if skip_nul == NA_INTEGER {
        return Err(cx.error("invalid 'skipNul' argument"));
    }
    let skip_nul = skip_nul != 0;
    let warn = warn != 0;

    let wasopen = flags(cx, ncon)?.isopen;
    if !wasopen {
        // UTF8out: a request (the open's set_iconv drops it)
        with_conn(cx, ncon, |c| c.c.utf8out = 1)?;
        if !open_with_mode(cx, ncon, "rt")? {
            return Err(cx.error("cannot open the connection"));
        }
    }
    let ours = our_fgetc(cx);
    let utf8locale = cx.locale().utf8;
    // the loop; an error in it closes a connection readLines opened (the
    // context's cend, checkClose)
    let body = (|| -> RResult<(Vec<Vec<u8>>, Encoding, Option<Vec<u8>>)> {
        let f = flags(cx, ncon)?;
        if !f.canread {
            return Err(cx.error("cannot read from this connection"));
        }
        if wasopen {
            // for a non-blocking connection, more input may have become
            // available, so re-position
            let r = with_conn(cx, ncon, |c| {
                if c.canseek() && !c.blocking() {
                    let pos = rconn_seek(c, -1.0, 1, 1)?;
                    rconn_seek(c, pos, 1, 1)?;
                }
                Ok::<(), String>(())
            })?;
            r.map_err(|m| cx.error(&m))?;
        }
        let utf8out = with_conn(cx, ncon, |c| {
            c.set_incomplete(false);
            c.c.utf8out != 0
        })?;
        let oenc = if utf8out || encoding == b"UTF-8" {
            Encoding::Utf8
        } else if encoding == b"latin1" {
            Encoding::Latin1
        } else if encoding == b"bytes" {
            Encoding::Bytes
        } else {
            Encoding::Native
        };
        let nnn = if n < 0 { i64::MAX } else { n };
        let mut lines = Vec::new();
        let mut nread: i64 = 0;
        while nread < nnn {
            let r = with_conn(cx, ncon, |c| read_line(c, skip_nul, ours))?;
            let (buf, ch) = r.map_err(|m| cx.error(&m))?;
            let len = strlen(&buf);
            // Remove UTF-8 BOM
            let from = if nread == 0 && utf8locale && len >= 3 && buf[..3] == *b"\xef\xbb\xbf" { 3 } else { 0 };
            lines.push(buf[from..len].to_vec());
            if warn && len < buf.len() {
                cx.warning(&format!("line {} appears to contain an embedded nul", nread + 1))?;
            }
            if ch == R_EOF {
                // no more lines: the last is incomplete, or none
                if buf.is_empty() {
                    lines.pop();
                }
                return Ok((lines, oenc, (!buf.is_empty()).then_some(buf)));
            }
            nread += 1;
        }
        Ok((lines, oenc, None))
    })();
    if !wasopen {
        let err = with_conn(cx, ncon, close_conn)?;
        if body.is_err()
            && let Some(e) = err
        {
            cx.warning(&format!("Problem closing connection:  {e}"))?;
        }
    }
    let (mut lines, oenc, last) = body?;
    let nnn = if n < 0 { i64::MAX } else { n };
    if let Some(buf) = last {
        // incomplete last line
        let (text, blocking, class, desc) =
            with_conn(cx, ncon, |c| (c.text(), c.blocking(), c.class, c.description.clone()))?;
        if text && !blocking && class != "gzfile" {
            // push back the rest
            lines.pop();
            let r = with_conn(cx, ncon, |c| {
                let r = c.push_back_line(&buf, false);
                c.set_incomplete(true);
                r
            })?;
            r.map_err(|m| cx.error(&m))?;
        } else if warn {
            let d = String::from_utf8_lossy(&desc).into_owned();
            cx.warning(&format!("incomplete final line found on '{d}'"))?;
        }
    }
    if (lines.len() as i64) < nnn && ok == 0 {
        return Err(cx.error("too few lines read in readLines"));
    }
    cx.build(out, |cx| {
        let mut v = cx.alloc_vector::<Char<'_>>(lines.len())?;
        for (i, l) in lines.iter().enumerate() {
            v.set(i, cx.mk_char(l, oenc)?);
        }
        Ok(v.finish().into())
    })
}

/// One `con->read(p, 1, buf.len(), con)` of connection `ncon`: how many
/// bytes it read (a read the system fails reads none, as the C's fread
/// does). The connection's own read, past its text buffer and pushback.
fn read_bytes(cx: &mut Cx, ncon: i32, buf: &mut [u8]) -> RResult<usize> {
    let r = with_conn(cx, ncon, |c| match super::conn_read_raw(c, buf) {
        Ok(k) => Ok(k),
        Err(e) if e.raw_os_error().is_none() => Err(e.to_string()),
        Err(_) => Ok(0),
    })?;
    r.map_err(|m| cx.error(&m))
}

/// `readFixedString(con, len, useBytes)`: `len` characters (bytes when
/// `!utf8`) from the connection, cut at the first NUL (the warning given
/// as the NUL is read, once per call); `None` when nothing could be read.
// R: connections.c:readFixedString
fn read_fixed_string(
    cx: &mut Cx,
    ncon: i32,
    len: usize,
    utf8: bool,
    warn_on_nul: &mut bool,
) -> RResult<Option<Vec<u8>>> {
    let mut buf = Vec::new();
    if utf8 {
        for i in 0..len {
            let mut b = [0u8; 1];
            if read_bytes(cx, ncon, &mut b)? == 0 {
                if i == 0 {
                    return Ok(None);
                }
                break;
            }
            let clen = utf8clen(b[0]);
            buf.push(b[0]);
            if clen > 1 {
                let mut rest = vec![0u8; clen - 1];
                let m = read_bytes(cx, ncon, &mut rest)?;
                if m < clen - 1 {
                    return Err(cx.error("invalid UTF-8 input in readChar()"));
                }
                let q = buf.len() - 1;
                buf.extend_from_slice(&rest);
                // NB: this only checks validity of multi-byte characters
                if robigo_text::mbrtowc_utf8(&buf[q..], 0).0 < 0 {
                    return Err(cx.error("invalid UTF-8 input in readChar()"));
                }
            } else if b[0] == 0 && *warn_on_nul {
                *warn_on_nul = false;
                cx.warning("truncating string with embedded nuls")?;
            }
        }
    } else {
        buf.resize(len, 0);
        let m = read_bytes(cx, ncon, &mut buf)?;
        if len > 0 && m == 0 {
            return Ok(None);
        }
        buf.truncate(m);
        if buf.contains(&0) && *warn_on_nul {
            *warn_on_nul = false;
            cx.warning("truncating string with embedded nuls")?;
        }
    }
    // the string ends at its first NUL (R >= 2.8.0 takes NULs to be padding)
    if let Some(z) = buf.iter().position(|&b| b == 0) {
        buf.truncate(z);
    }
    Ok(Some(buf))
}

/// `rawFixedString(bytes, len, nbytes, &np, useBytes)`: the next `len`
/// characters (bytes when `!utf8`) of `bytes` from `*np`; `None` at the end.
// R: connections.c:rawFixedString
fn raw_fixed_string(bytes: &[u8], len: usize, np: &mut usize, utf8: bool) -> Result<Option<Vec<u8>>, String> {
    let nbytes = bytes.len();
    let mut len = len;
    if *np + len > nbytes {
        len = nbytes - *np;
        if len == 0 {
            return Ok(None);
        }
    }
    if utf8 {
        let mut iread = *np;
        for _ in 0..len {
            if iread >= nbytes {
                break;
            }
            let clen = utf8clen(bytes[iread]);
            if iread + clen > nbytes {
                return Err("invalid UTF-8 input in readChar()".into());
            }
            iread += clen;
        }
        let r = bytes[*np..iread].to_vec();
        *np = iread;
        Ok(Some(r))
    } else {
        let r = bytes[*np..*np + len].to_vec();
        *np += len;
        Ok(Some(r))
    }
}

/// `utf8clen(c)`: the length of the UTF-8 character `c` starts (1 for a
/// byte that starts none).
// R: util.c:utf8clen
fn utf8clen(c: u8) -> usize {
    if c & 0xc0 != 0xc0 {
        return 1;
    }
    const T4: [u8; 64] = [
        1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2,
        2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5,
    ];
    1 + T4[(c & 0x3f) as usize] as usize
}

/// `readChar(con, nchars, useBytes)`: per element of `nchars` that many
/// characters (bytes with `useBytes`) of the connection or raw vector, as
/// long as there are any.
// R: connections.c:do_readchar
pub fn do_readchar(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "readChar", 3, true)?;
    let raw: Option<Vec<u8>> = match args.get(cx, 0).view() {
        View::Raw(v) => match v.as_slice() {
            Some(b) => Some(b.to_vec()),
            None => return Err(cx.error("robigo: readChar() of a package ALTREP raw vector is not implemented yet")),
        },
        _ => None,
    };
    let ncon = if raw.is_none() {
        let n = as_integer_warn(cx, args.root(0))?;
        if !with_conn(cx, n, |c| c.canread())? {
            return Err(cx.error("cannot read from this connection"));
        }
        n
    } else {
        0
    };
    // We did as.integer in the wrapper
    // (XLENGTH, then INTEGER: an integer vector)
    let nchars: Vec<i32> = match args.get(cx, 1).view() {
        View::Int(v) => match v.as_slice() {
            Some(b) => b.to_vec(),
            None => return Err(cx.error("robigo: readChar() of a package ALTREP 'nchars' is not implemented yet")),
        },
        _ if args.get(cx, 1).len() == Some(0) => Vec::new(),
        _ => return Err(cx.error("robigo: readChar() with a non-integer 'nchars' is not implemented yet")),
    };
    if nchars.is_empty() {
        cx.build(out, |cx| Ok(cx.alloc_vector::<Char<'_>>(0)?.finish().into()))?;
        return Ok(());
    }
    let use_bytes = as_logical(cx, args.get(cx, 2));
    if use_bytes == NA_INTEGER {
        return Err(cx.error("invalid 'useBytes' argument"));
    }
    let utf8 = cx.locale().utf8 && use_bytes == 0;
    let wasopen = raw.is_some() || flags(cx, ncon)?.isopen;
    if !wasopen {
        // Documented behaviour
        if !open_with_mode(cx, ncon, "rb")? {
            return Err(cx.error("cannot open the connection"));
        }
    }
    // (mbcslocale && !utf8locale: robigo-rt's locales are UTF-8 or ASCII)
    // the loop; an error in it closes a connection readChar opened (the
    // context's cend, con_cleanup's checkClose)
    let body = (|| -> RResult<Vec<Vec<u8>>> {
        if raw.is_none() {
            let (canread, warn) = with_conn(cx, ncon, |c| {
                (c.canread(), c.text() && (!c.c.buff.is_null() || c.c.n_push_back >= 0 || !c.c.inconv.is_null()))
            })?;
            if !canread {
                return Err(cx.error("cannot read from this connection"));
            }
            if warn {
                // could be turned into runtime error
                cx.warning("text connection used with readChar(), results may be incorrect")?;
            }
        }
        let mut res = Vec::new();
        let (mut np, mut warn_on_nul) = (0usize, true);
        for &len in &nchars {
            if len == NA_INTEGER || len < 0 {
                return Err(cx.error("invalid 'nchars' argument"));
            }
            let one = match &raw {
                Some(b) => raw_fixed_string(b, len as usize, &mut np, utf8).map_err(|m| cx.error(&m))?,
                None => read_fixed_string(cx, ncon, len as usize, utf8, &mut warn_on_nul)?,
            };
            let Some(one) = one else { break };
            if one.contains(&0) {
                // mkCharLenCE's error, at this element
                let s = cx.scope(|cx, s| {
                    let o = s.out(cx);
                    cx.build(o, |cx| {
                        let mut v = cx.alloc_vector::<Char<'_>>(1)?;
                        v.set(0, cx.mk_char(&one, Encoding::Native)?);
                        Ok(v.finish().into())
                    })
                });
                s?;
            }
            res.push(one);
        }
        Ok(res)
    })();
    if !wasopen {
        let err = with_conn(cx, ncon, close_conn)?;
        if body.is_err()
            && let Some(e) = err
        {
            cx.warning(&format!("Problem closing connection:  {e}"))?;
        }
    }
    let res = body?;
    cx.build(out, |cx| {
        let mut v = cx.alloc_vector::<Char<'_>>(res.len())?;
        for (i, s) in res.iter().enumerate() {
            v.set(i, cx.mk_char(s, Encoding::Native)?);
        }
        Ok(v.finish().into())
    })
}
// ---- dcf.c ---------------------------------------------------------------------

/// dcf.c's patterns (TRE, on bytes), as the byte tests they are.
mod dcf {
    pub fn blank(c: u8) -> bool {
        c == b' ' || c == b'\t'
    }
    fn space(c: u8) -> bool {
        matches!(c, b' ' | b'\x0c' | b'\n' | b'\r' | b'\t' | b'\x0b')
    }
    /// `^[ \t]*$`
    pub fn blankline(l: &[u8]) -> bool {
        l.iter().all(|&c| blank(c))
    }
    /// `^[ \t]+`: the match's end, if it matches.
    pub fn contline(l: &[u8]) -> Option<usize> {
        let n = l.iter().take_while(|&&c| blank(c)).count();
        (n > 0).then_some(n)
    }
    /// `[ \t]+$`: the match's start (the trailing run of blanks), if any.
    pub fn trailblank(l: &[u8]) -> Option<usize> {
        let n = l.iter().rev().take_while(|&&c| blank(c)).count();
        (n > 0).then_some(l.len() - n)
    }
    /// `^[^:]+:[ \t]*`: the match's end, if it matches.
    pub fn regline(l: &[u8]) -> Option<usize> {
        let colon = l.iter().position(|&c| c == b':')?;
        if colon == 0 {
            return None;
        }
        Some(colon + 1 + l[colon + 1..].iter().take_while(|&&c| blank(c)).count())
    }
    /// `^[ \f\n\r\t\v]+\.[ \f\n\r\t\v]*$`
    pub fn eblankline(l: &[u8]) -> bool {
        let n = l.iter().take_while(|&&c| space(c)).count();
        n > 0 && l.get(n) == Some(&b'.') && l[n + 1..].iter().all(|&c| space(c))
    }
}

/// A line of a DCF message: `line[20] = '\0'`, the first 20 bytes (or
/// fewer).
fn dcf_head(l: &[u8]) -> String {
    String::from_utf8_lossy(&l[..l.len().min(20)]).into_owned()
}

/// `readDCF(file, fields, keepwhite)`: the records of a Debian Control
/// File as a character matrix, a record per row and a field per column
/// (`fields`, or every field seen when it is empty), NA where a record has
/// none; fields not in `keepwhite` folded (their continuation lines' and
/// trailing white space dropped, a " ." line a blank line).
// R: dcf.c:do_readDCF
pub fn do_readdcf(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "readDCF", 3, true)?;
    let ncon = as_integer_warn(cx, args.root(0))?;
    let wasopen = flags(cx, ncon)?.isopen;
    if !wasopen && !super::open(cx, ncon)? {
        return Err(cx.error("cannot open the connection"));
    }
    let ours = our_fgetc(cx);
    // the work; an error in it closes a connection readDCF opened (the
    // context's cend, con_cleanup)
    let body = (|| -> RResult<(Vec<Vec<u8>>, Vec<Vec<Option<Vec<u8>>>>)> {
        if !flags(cx, ncon)?.canread {
            return Err(cx.error("cannot read from this connection"));
        }
        let strings = |cx: &mut Cx, i: usize| -> RResult<Vec<Vec<u8>>> {
            cx.scope(|cx, s| {
                let v = s.capture(cx, |cx, o| {
                    robigo_rtbuiltins::coerce::vector::coerce_vector(cx, args.root(i), STRSXP, o)
                })?;
                let v = v.bind(cx);
                let Some(v) = v.as_vector::<Char<'_>>() else {
                    return Err(robigo_rtbuiltins::util::not_yet(
                        cx,
                        Some(call),
                        "readDCF() of a package ALTREP character vector",
                    ));
                };
                Ok((0..v.len()).map(|k| v.get(k).bytes().to_vec()).collect())
            })
        };
        let mut what = strings(cx, 1)?;
        let fold_excludes = strings(cx, 2)?;
        let dynwhat = what.is_empty();
        let foldable = |f: &[u8]| !fold_excludes.iter().any(|e| e.as_slice() == f);
        // a record per entry, a field per position (NA: None)
        let mut recs: Vec<Vec<Option<Vec<u8>>>> = vec![vec![None; what.len()]];
        let mut lastm: Option<usize> = None;
        let mut blank_skip = true;
        let mut field_skip = false;
        let mut field_fold = true;
        let mut n_eblanklines = 0usize;
        loop {
            let r = with_conn(cx, ncon, |c| read_line(c, false, ours))?;
            let (buf, ch) = r.map_err(|m| cx.error(&m))?;
            // Rconn_getline2: no line at the end of input; a C string
            if ch == R_EOF && buf.is_empty() {
                break;
            }
            let line = &buf[..strlen(&buf)];
            if line.is_empty() || dcf::blankline(line) {
                // a blank line: the first one after a record ends it, later
                // ones are skipped
                if !blank_skip {
                    recs.push(vec![None; what.len()]);
                    blank_skip = true;
                    lastm = None;
                    field_skip = false;
                    field_fold = true;
                    n_eblanklines = 0;
                }
            } else if line[0] == b'#' {
                // a comment line
            } else {
                blank_skip = false;
                if let Some(lead) = dcf::contline(line) {
                    // a continuation line: wrong at the start of a record
                    if lastm.is_none() && !field_skip {
                        let m =
                            format!("Found continuation line starting '{} ...' at begin of record.", dcf_head(line));
                        return Err(cx.error(&m));
                    }
                    if let Some(m) = lastm {
                        let k = recs.len() - 1;
                        let is_eblankline = dcf::eblankline(line);
                        if is_eblankline && field_fold {
                            n_eblanklines += 1;
                            if ch == R_EOF {
                                break;
                            }
                            continue;
                        }
                        let rest: &[u8] = if is_eblankline {
                            b""
                        } else if field_fold {
                            let end = dcf::trailblank(line).unwrap_or(line.len());
                            &line[lead.min(end)..end]
                        } else {
                            line
                        };
                        let mut v = recs[k][m].clone().unwrap_or_else(|| b"NA".to_vec());
                        if !v.is_empty() || !field_fold {
                            v.push(b'\n');
                        }
                        if !is_eblankline {
                            v.extend(std::iter::repeat_n(b'\n', n_eblanklines));
                            n_eblanklines = 0;
                            v.extend_from_slice(rest);
                        }
                        recs[k][m] = Some(v);
                    }
                } else if let Some(off) = dcf::regline(line) {
                    let k = recs.len() - 1;
                    for (m, w) in what.iter().enumerate() {
                        if line.len() > w.len() && line[w.len()] == b':' && line.starts_with(w) {
                            // a field being recorded
                            lastm = Some(m);
                            field_skip = false;
                            if !fold_excludes.is_empty() {
                                field_fold = foldable(w);
                            }
                            let end = if field_fold { dcf::trailblank(line).unwrap_or(line.len()) } else { line.len() };
                            recs[k][m] = Some(line[off.min(end)..end].to_vec());
                            break;
                        } else {
                            // a field, not one asked for
                            lastm = None;
                            field_skip = true;
                        }
                    }
                    if dynwhat && lastm.is_none() {
                        // a field not seen before, recording all of them
                        field_skip = false;
                        let colon = line.iter().position(|&c| c == b':').expect("regline matched");
                        what.push(line[..colon].to_vec());
                        for r in recs.iter_mut() {
                            r.push(None);
                        }
                        let m = what.len() - 1;
                        lastm = Some(m);
                        if !fold_excludes.is_empty() {
                            field_fold = foldable(&what[m]);
                        }
                        let end = if field_fold { dcf::trailblank(line).unwrap_or(line.len()) } else { line.len() };
                        recs[k][m] = Some(line[off.min(end)..end].to_vec());
                    }
                } else {
                    // a regular line with no tag
                    return Err(cx.error(&format!("Line starting '{} ...' is malformed!", dcf_head(line))));
                }
            }
            if ch == R_EOF {
                break;
            }
        }
        if blank_skip {
            recs.pop();
        }
        Ok((what, recs))
    })();
    if !wasopen {
        with_conn(cx, ncon, |c| {
            if c.isopen() {
                close_conn(c);
            }
        })?;
    }
    let (what, recs) = body?;
    // the transpose: a row per record, a column per field
    let (nr, nc) = (recs.len(), what.len());
    cx.build(out, |cx| {
        let mut v = cx.alloc_vector::<Char<'_>>(nr * nc)?;
        for (k, r) in recs.iter().enumerate() {
            for (m, x) in r.iter().enumerate() {
                let c = match x {
                    Some(b) => cx.mk_char(b, Encoding::Native)?,
                    None => cx.na_string(),
                };
                v.set(k + nr * m, c);
            }
        }
        let mut names = cx.alloc_vector::<Char<'_>>(nc)?;
        for (m, w) in what.iter().enumerate() {
            names.set(m, cx.mk_char(w, Encoding::Native)?);
        }
        let mut dn = cx.alloc_vector::<robigo_rtcore::Ref<'_>>(2)?;
        dn.set(0, cx.nil());
        // (dimnamesgets: an empty element is NULL)
        if nc > 0 {
            dn.set(1, names.finish().into());
        } else {
            dn.set(1, cx.nil());
        }
        let mut a = robigo_rtbuiltins::attrib::Attrs::new();
        a.install(robigo_rtbuiltins::attrib::Attr {
            tag: cx.sym(robigo_rtcore::RSym::Dim),
            value: cx.alloc_copy(&[nr as i32, nc as i32])?.finish().into(),
        });
        a.install(robigo_rtbuiltins::attrib::Attr {
            tag: cx.sym(robigo_rtcore::RSym::DimNames),
            value: dn.finish().into(),
        });
        a.apply(cx, &mut v)?;
        Ok(v.finish().into())
    })
}

// ---- text connections ------------------------------------------------------------

/// `translateChar` (`ty` 1), `translateCharUTF8` (3) or `CHAR` (any
/// other) of a string, as text_init reads them.
/// `None` for a "bytes" string that `ty` translates (GNU R's error:
/// [`BYTES_REFUSED`]).
fn text_bytes(cx: &Cx, c: Char<'_>, ty: i32) -> Option<Vec<u8>> {
    if c == cx.na_string() || (ty != 1 && ty != 3) {
        return Some(c.bytes().to_vec());
    }
    let s = robigo_rtbuiltins::coerce::text(c);
    let loc = cx.locale();
    let r = if ty == 1 { robigo_rtbuiltins::text::to_native(s, loc) } else { robigo_rtbuiltins::text::to_utf8(s, loc) };
    r.ok().map(|b| b.into_owned())
}

/// translateChar's error for a "bytes" string.
const BYTES_REFUSED: &str = "translating strings with \"bytes\" encoding is not allowed";

/// `textConnection(description, object, open, env, type)`: an input text
/// connection over the strings of `object`, each followed by a newline,
/// already open. Output text connections (`open` "w" or "a") are not
/// implemented yet.
// R: connections.c:do_textconnection, newtext, text_init
pub fn do_textconnection(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "textConnection", 5, true)?;
    let desc = match args.get(cx, 0).as_vector::<Char<'_>>() {
        Some(v) if v.len() == 1 && v.get(0) != cx.na_string() => Some(v.get(0).bytes().to_vec()),
        _ => None,
    };
    let Some(_) = desc else {
        return Err(cx.error("invalid 'description' argument"));
    };
    let desc = text_bytes(cx, args.get(cx, 0).as_vector::<Char<'_>>().unwrap().get(0), 1);
    let Some(desc) = desc else { return Err(cx.error(BYTES_REFUSED)) };
    let open = match args.get(cx, 2).as_vector::<Char<'_>>() {
        Some(v) if v.len() == 1 => v.get(0).bytes().to_vec(),
        _ => return Err(cx.error("invalid 'open' argument")),
    };
    let venv = args.get(cx, 3);
    if venv.sexptype() == robigo_heap::types::NILSXP {
        return Err(cx.error("use of NULL environment is defunct"));
    }
    if venv.sexptype() != robigo_heap::types::ENVSXP {
        return Err(cx.error("invalid 'environment' argument"));
    }
    let ty = as_integer_warn(cx, args.root(4))?;
    if ty == NA_INTEGER {
        return Err(cx.error("invalid 'encoding' argument"));
    }
    let ncon = super::next_connection(cx)?;
    if open.is_empty() || open[0] == b'r' {
        if args.get(cx, 1).sexptype() != STRSXP {
            return Err(cx.error("invalid 'text' argument"));
        }
        let mut data = Vec::new();
        let ok = args.get(cx, 1).as_vector::<Char<'_>>().map(|v| {
            (0..v.len()).all(|i| match text_bytes(cx, v.get(i), ty) {
                // (strlen: up to a NUL, which a CHARSXP has none of)
                Some(b) => {
                    data.extend_from_slice(&b);
                    data.push(b'\n');
                    true
                }
                None => false,
            })
        });
        match ok {
            None => {
                return Err(robigo_rtbuiltins::util::not_yet(
                    cx,
                    Some(call),
                    "textConnection() of a package ALTREP character vector",
                ));
            }
            Some(false) => return Err(cx.error(BYTES_REFUSED)),
            Some(true) => {}
        }
        let mut con = Conn::new("textConnection", &desc, "r", super::Imp::Text(super::TextConn { data, cur: 0 }));
        con.set_isopen(true);
        con.set_canwrite(false);
        super::publish(cx, ncon, con, "textConnection", out)
    } else if open[0] == b'w' || open[0] == b'a' {
        Err(robigo_rtbuiltins::util::not_yet(cx, Some(call), "an output textConnection (open = \"w\" or \"a\")"))
    } else {
        Err(cx.error("unsupported mode"))
    }
}
