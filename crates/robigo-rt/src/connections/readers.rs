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
                let v = s.capture(cx, |cx, o| robigo_rtbuiltins::coerce::vector::coerce_vector(cx, args.root(i), STRSXP, o))?;
                let v = v.bind(cx);
                let Some(v) = v.as_vector::<Char<'_>>() else {
                    return Err(robigo_rtbuiltins::util::not_yet(cx, Some(call), "readDCF() of a package ALTREP character vector"));
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
                        let m = format!("Found continuation line starting '{} ...' at begin of record.", dcf_head(line));
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
        dn.set(1, names.finish().into());
        let mut a = robigo_rtbuiltins::attrib::Attrs::new();
        a.install(robigo_rtbuiltins::attrib::Attr {
            tag: cx.sym(robigo_rtcore::RSym::Dim),
            value: cx.alloc_copy(&[nr as i32, nc as i32])?.finish().into(),
        });
        a.install(robigo_rtbuiltins::attrib::Attr { tag: cx.sym(robigo_rtcore::RSym::DimNames), value: dn.finish().into() });
        a.apply(cx, &mut v)?;
        Ok(v.finish().into())
    })
}
