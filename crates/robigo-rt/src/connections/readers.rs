//! connections.c's text readers (L4): `isOpen`, `isIncomplete`, the
//! pushback (`pushBack`, `pushBackLength`, `clearPushBack`) and
//! `readLines`. They read through [`conn_fgetc`] (`Rconn_fgetc`), as GNU
//! R's do.

use super::{Conn, NA_INTEGER, R_EOF, close_conn, conn_fgetc, conn_number, flags, open_with_mode, our_fgetc, with_conn};
use robigo_heap::types::STRSXP;
use robigo_rtbuiltins::coerce::vector::as_integer_warn;
use robigo_rtbuiltins::util::as_logical;
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
            return Err(robigo_rtbuiltins::util::not_yet(cx, Some(call), "pushBack() of a package ALTREP character vector"));
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
            let reseek = with_conn(cx, ncon, |c| c.canseek() && !c.blocking())?;
            if reseek {
                return Err(robigo_rtbuiltins::util::not_yet(
                    cx,
                    Some(call),
                    "readLines() of an open non-blocking seekable connection",
                ));
            }
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
