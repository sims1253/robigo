//! scan.c's `scan()` for character data (L4): `scanchar` (comments,
//! escapes), `fillBuffer` (white space or a separator, quotes, stripping),
//! `scanVector` and `scanFrame` (fill, flush, multi.line), and `do_scan`'s
//! arguments. What GNU R's build reads with it is text: `what` a character
//! vector or a list of character vectors and NULLs. Other types of `what`
//! (whose items are converted with `R_strtod` and friends) and the console
//! (`file = ""`) say they are not implemented yet.
//!
//! The reading runs with the connection borrowed (`Rconn_fgetc` per
//! character); its warnings are given after it, in the order they arose,
//! then its error if it stopped at one.

use super::{Conn, NA_INTEGER, R_EOF, close_conn, conn_fgetc, flags, our_fgetc, with_conn};
use robigo_heap::types::{LGLSXP, NILSXP, STRSXP, VECSXP};
use robigo_rtbuiltins::util::as_logical;
use robigo_rtcore::{Args, Char, Cx, Encoding, Out, RResult, Ref, Root, View};
use std::ffi::c_int;

/// What a column holds: text, or nothing (a NULL in a `what` list).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ty {
    Str,
    Nil,
}

/// scan.c's `LocalData`, the parts robigo reads.
struct Local<'a> {
    na_strings: &'a [Vec<u8>],
    sepchar: c_int,
    quoteset: Vec<u8>,
    /// `NO_COMCHAR` is `None`.
    comchar: Option<c_int>,
    escapes: bool,
    /// an unscanchar-ed character (0: none)
    save: c_int,
    at_start: bool,
    embed_warn: bool,
    skip_nul: bool,
    utf8locale: bool,
    ours: Option<usize>,
    warnings: Vec<&'static str>,
}

/// `strchr(quoteset, c)`: C converts `c` to a char, and the terminator
/// matches 0.
fn in_set(set: &[u8], c: c_int) -> bool {
    let b = c as u8;
    b == 0 || set.contains(&b)
}

/// `Rspace(c)` (not a Latin-1 locale: robigo has none).
fn rspace(c: c_int) -> bool {
    c == b' ' as c_int || c == b'\t' as c_int || c == b'\n' as c_int || c == b'\r' as c_int
}

/// `strlen`.
fn strlen(b: &[u8]) -> usize {
    b.iter().position(|&x| x == 0).unwrap_or(b.len())
}

impl Local<'_> {
    // R: scan.c:scanchar_raw (not the console)
    fn raw(&mut self, con: &mut Conn) -> Result<c_int, String> {
        let mut c = conn_fgetc(con, self.ours)?;
        if c == 0 {
            if self.skip_nul {
                while c == 0 {
                    c = conn_fgetc(con, self.ours)?;
                }
            } else {
                self.embed_warn = true;
            }
        }
        Ok(c)
    }

    // R: scan.c:scanchar
    fn scanchar(&mut self, in_quote: bool, con: &mut Conn) -> Result<c_int, String> {
        let mut next = if self.save != 0 {
            let s = self.save;
            self.save = 0;
            s
        } else {
            self.raw(con)?
        };
        if Some(next) == self.comchar && !in_quote {
            loop {
                next = self.raw(con)?;
                if next == b'\n' as c_int || next == R_EOF {
                    break;
                }
            }
        }
        if next == b'\\' as c_int && self.escapes {
            next = self.raw(con)?;
            let oct = |c: c_int| (b'0' as c_int..=b'8' as c_int).contains(&c);
            if oct(next) {
                let mut octal = next - b'0' as c_int;
                next = self.raw(con)?;
                if oct(next) {
                    octal = 8 * octal + next - b'0' as c_int;
                    next = self.raw(con)?;
                    if oct(next) {
                        octal = 8 * octal + next - b'0' as c_int;
                    } else {
                        self.save = next;
                    }
                } else {
                    self.save = next;
                }
                next = octal;
            } else {
                next = match u8::try_from(next).ok() {
                    Some(b'a') => 0x07,
                    Some(b'b') => 0x08,
                    Some(b'f') => 0x0c,
                    Some(b'n') => b'\n' as c_int,
                    Some(b'r') => b'\r' as c_int,
                    Some(b't') => b'\t' as c_int,
                    Some(b'v') => 0x0b,
                    Some(b'x') => {
                        let mut val = 0;
                        for _ in 0..2 {
                            let c = self.raw(con)?;
                            let ext = match u8::try_from(c).ok() {
                                Some(d @ b'0'..=b'9') => (d - b'0') as c_int,
                                Some(d @ b'A'..=b'F') => (d - b'A' + 10) as c_int,
                                Some(d @ b'a'..=b'f') => (d - b'a' + 10) as c_int,
                                _ => {
                                    self.save = c;
                                    break;
                                }
                            };
                            val = 16 * val + ext;
                        }
                        val
                    }
                    // any other character, and even EOF, escapes to itself,
                    // but \" etc. inside quotes are kept
                    _ => {
                        if in_quote && in_set(&self.quoteset, next) {
                            self.save = next;
                            b'\\' as c_int
                        } else {
                            next
                        }
                    }
                };
            }
        }
        Ok(next)
    }

    /// `fillBuffer(type, strip, &bch, d, buffer)`: the next item's bytes
    /// (up to the first NUL, as C reads them later) and `bch`.
    // R: scan.c:fillBuffer
    fn fill_buffer(&mut self, ty: Ty, mut strip: bool, con: &mut Conn) -> Result<(Vec<u8>, c_int), String> {
        let mut buf: Vec<u8> = Vec::new();
        let mut mm = 0;
        let mut filled: c_int = 1;
        let nl = b'\n' as c_int;
        let cr = b'\r' as c_int;
        let blank = |c: c_int| c == b' ' as c_int || c == b'\t' as c_int;
        'done: {
            if self.sepchar == 0 {
                // skip all space or tabs
                strip = false;
                let mut c = self.scanchar(false, con)?;
                while blank(c) {
                    c = self.scanchar(false, con)?;
                }
                if c == nl || c == cr || c == R_EOF {
                    filled = c;
                    break 'done;
                }
                if ty == Ty::Str && in_set(&self.quoteset, c) {
                    let quote = c;
                    loop {
                        c = self.scanchar(true, con)?;
                        if c == R_EOF || c == quote {
                            break;
                        }
                        if c == b'\\' as c_int {
                            // an embedded quote is unquoted; other
                            // backslashes are kept
                            c = self.scanchar(true, con)?;
                            if c == R_EOF {
                                break;
                            }
                            if c != quote {
                                buf.push(b'\\');
                            }
                        }
                        buf.push(c as u8);
                    }
                    if c == R_EOF {
                        self.warnings.push("EOF within quoted string");
                    }
                    c = self.scanchar(false, con)?;
                    mm = buf.len();
                } else {
                    // not a quoted string
                    loop {
                        buf.push(c as u8);
                        c = self.scanchar(false, con)?;
                        if rspace(c) || c == R_EOF {
                            break;
                        }
                    }
                }
                while blank(c) {
                    c = self.scanchar(false, con)?;
                }
                if c == nl || c == cr || c == R_EOF {
                    filled = c;
                } else {
                    self.save = c;
                }
            } else {
                // a separator
                let mut c;
                loop {
                    c = self.scanchar(false, con)?;
                    if c == self.sepchar || c == nl || c == cr || c == R_EOF {
                        break;
                    }
                    // eat white space
                    if ty != Ty::Str {
                        while blank(c) {
                            c = self.scanchar(false, con)?;
                            if c == self.sepchar || c == nl || c == cr || c == R_EOF {
                                filled = c;
                                break 'done;
                            }
                        }
                    }
                    // CSV style quoted string handling
                    if ty == Ty::Str && c != 0 && in_set(&self.quoteset, c) {
                        let quote = c;
                        loop {
                            // inquote:
                            loop {
                                c = self.scanchar(true, con)?;
                                if c == R_EOF || c == quote {
                                    break;
                                }
                                buf.push(c as u8);
                            }
                            if c == R_EOF {
                                self.warnings.push("EOF within quoted string");
                            }
                            c = self.scanchar(true, con)?;
                            if c == quote {
                                buf.push(quote as u8);
                                continue;
                            }
                            break;
                        }
                        mm = buf.len();
                        if c == self.sepchar || c == nl || c == cr || c == R_EOF {
                            filled = c;
                            break 'done;
                        }
                        self.save = c;
                        continue;
                    }
                    if !strip || !buf.is_empty() || !rspace(c) {
                        buf.push(c as u8);
                    }
                }
                filled = c;
            }
        }
        // strip trailing white space, if desired and if the item is not null
        let mut end = buf.len();
        if strip && buf.len() > mm {
            let mut m = buf.len() as isize;
            let mut p = buf.len() as isize;
            loop {
                p -= 1;
                let c = if p >= 0 { buf[p as usize] as c_int } else { 0 };
                let more = m > mm as isize;
                m -= 1;
                if !(more && rspace(c)) {
                    break;
                }
            }
            end = (p + 1).max(0) as usize;
        }
        buf.truncate(end);
        let len = strlen(&buf);
        buf.truncate(len);
        // Remove UTF-8 BOM
        if self.at_start && self.utf8locale && buf.starts_with(b"\xef\xbb\xbf") {
            buf.drain(..3);
        }
        self.at_start = false;
        Ok((buf, filled))
    }

    /// `extractItem` of text: NA when `na.strings` has it.
    // R: scan.c:extractItem, isNAstring (mode 1)
    fn item(&self, b: &[u8]) -> Option<Vec<u8>> {
        if self.na_strings.iter().any(|n| n.as_slice() == b) { None } else { Some(b.to_vec()) }
    }
}

/// What scan read: a vector, or a list of columns (`None`: NULL), and the
/// count the message reports.
enum Read {
    Vector(Vec<Option<Vec<u8>>>),
    /// The columns and the records read.
    Frame(Vec<Option<Vec<Option<Vec<u8>>>>>, i64),
}

// R: scan.c:scanVector (STRSXP, not the console)
fn scan_vector(
    d: &mut Local<'_>,
    con: &mut Conn,
    maxitems: i64,
    maxlines: i64,
    flush: bool,
    strip: bool,
    blskip: bool,
) -> Result<Vec<Option<Vec<u8>>>, String> {
    let mut ans = Vec::new();
    let (mut nprev, mut linesread) = (0usize, 0i64);
    let mut bch: c_int = 1;
    let nl = b'\n' as c_int;
    loop {
        if bch == R_EOF {
            break;
        } else if bch == nl {
            linesread += 1;
            if linesread == maxlines {
                break;
            }
            nprev = ans.len();
        }
        let (buf, b) = d.fill_buffer(Ty::Str, strip, con)?;
        bch = b;
        if nprev == ans.len() && buf.is_empty() && ((blskip && bch == nl) || bch == R_EOF) {
            if bch == R_EOF {
                break;
            }
        } else {
            ans.push(d.item(&buf));
            if ans.len() as i64 == maxitems {
                break;
            }
        }
        if flush && bch != nl && bch != R_EOF {
            let mut c;
            loop {
                c = d.scanchar(false, con)?;
                if c == nl || c == R_EOF {
                    break;
                }
            }
            bch = c;
        }
    }
    Ok(ans)
}

// R: scan.c:scanFrame (STRSXP and NILSXP columns, not the console)
#[allow(clippy::too_many_arguments)]
fn scan_frame(
    d: &mut Local<'_>,
    con: &mut Conn,
    cols: &[Ty],
    maxitems: i64,
    maxlines: i64,
    flush: bool,
    fill: bool,
    lstrip: &[i32],
    blskip: bool,
    multiline: bool,
) -> Result<(Vec<Option<Vec<Option<Vec<u8>>>>>, i64), String> {
    let nc = cols.len();
    let mut ans: Vec<Option<Vec<Option<Vec<u8>>>>> =
        cols.iter().map(|&t| (t == Ty::Str).then(Vec::new)).collect();
    let (mut n, mut linesread, mut colsread, mut ii) = (0i64, 0i64, 0usize, 0usize);
    let mut badline = 0i64;
    let mut bch: c_int = 1;
    let nl = b'\n' as c_int;
    let vec_strip = lstrip.len() == nc;
    let mut strip = lstrip[0] != 0;
    let extract = |d: &Local<'_>, ans: &mut Vec<Option<Vec<Option<Vec<u8>>>>>, ii: usize, b: &[u8]| {
        if let Some(col) = &mut ans[ii] {
            col.push(d.item(b));
        }
    };
    loop {
        if bch == R_EOF {
            break;
        } else if bch == nl {
            linesread += 1;
            if colsread != 0 {
                if fill {
                    for k in colsread..nc {
                        extract(d, &mut ans, k, b"");
                    }
                    n += 1;
                    ii = 0;
                    colsread = 0;
                } else if badline == 0 && !multiline {
                    badline = linesread;
                }
                if badline != 0 && !multiline {
                    return Err(format!("line {badline} did not have {nc} elements"));
                }
            }
            if maxitems > 0 && n >= maxitems {
                break;
            }
            if maxlines > 0 && linesread == maxlines {
                break;
            }
        }
        if vec_strip {
            strip = lstrip[colsread] != 0;
        }
        let (buf, b) = d.fill_buffer(cols[ii], strip, con)?;
        bch = b;
        if colsread == 0 && buf.is_empty() && ((blskip && bch == nl) || bch == R_EOF) {
            if bch == R_EOF {
                break;
            }
        } else {
            extract(d, &mut ans, ii, &buf);
            ii += 1;
            colsread += 1;
            // a row filled: the next
            if colsread == nc {
                n += 1;
                ii = 0;
                colsread = 0;
                if flush && bch != nl && bch != R_EOF {
                    let mut c;
                    loop {
                        c = d.scanchar(false, con)?;
                        if c == nl || c == R_EOF {
                            break;
                        }
                    }
                    bch = c;
                }
            }
        }
    }
    // done:
    if colsread != 0 {
        if !fill {
            d.warnings.push("number of items read is not a multiple of the number of columns");
        }
        // (an empty item: NA only if na.strings has "")
        for k in colsread..nc {
            extract(d, &mut ans, k, b"");
        }
        n += 1;
    }
    Ok((ans, n))
}

/// `asXLength(x)`: -999 for what is not a length.
// R: coerce.c:asXLength
fn as_xlength(cx: &mut Cx, x: Root<'_>) -> RResult<i64> {
    let r = x.bind(cx);
    let atomic = robigo_rtbuiltins::util::is_vector_atomic(r) && r.len().unwrap_or(0) >= 1;
    if !atomic {
        return Ok(-999);
    }
    if let View::Int(v) = r.view() {
        let i = v.get(0);
        return Ok(if i == NA_INTEGER { -999 } else { i as i64 });
    }
    if !matches!(r.view(), View::Lgl(_) | View::Real(_) | View::Cplx(_) | View::Str(_)) {
        return Err(cx.error("robigo: scan()'s count of a raw vector is not implemented yet"));
    }
    let d = robigo_rtbuiltins::coerce::vector::as_real_warn(cx, x)?;
    Ok(if !d.is_finite() || d > (1u64 << 52) as f64 || d < 0.0 { -999 } else { d as i64 })
}

/// A string argument's first element translated to the native encoding
/// (`translateChar(STRING_ELT(x, 0))`), the vector's length checked by
/// the caller.
fn first_native(cx: &Cx, x: Ref<'_>) -> Option<Vec<u8>> {
    let v = x.as_vector::<Char<'_>>()?;
    let c = v.get(0);
    if c == cx.na_string() {
        return Some(b"NA".to_vec());
    }
    robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), cx.locale()).ok().map(|b| b.into_owned())
}

fn strings(x: Ref<'_>) -> Vec<Vec<u8>> {
    match x.as_vector::<Char<'_>>() {
        Some(v) => (0..v.len()).map(|i| v.get(i).bytes().to_vec()).collect(),
        None => Vec::new(),
    }
}

/// `scan(file, what, nmax, sep, dec, quote, skip, nlines, na.strings,
/// flush, fill, strip.white, quiet, blank.lines.skip, multi.line,
/// comment.char, allowEscapes, encoding, skipNul)`.
// R: scan.c:do_scan
pub fn do_scan(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "scan", 19, true)?;
    let nmax = as_xlength(cx, args.root(2))?;
    let nskip = as_xlength(cx, args.root(6))?;
    let nlines = as_xlength(cx, args.root(7))?;
    let lg = |cx: &Cx, i: usize| as_logical(cx, args.get(cx, i));
    let flush = lg(cx, 9);
    let fill = lg(cx, 10);
    let mut quiet = lg(cx, 12);
    let mut blskip = lg(cx, 13);
    let mut multiline = lg(cx, 14);
    let escapes = lg(cx, 16);
    let encoding = match args.get(cx, 17).as_vector::<Char<'_>>() {
        Some(v) if v.len() == 1 => v.get(0).bytes().to_vec(),
        _ => return Err(cx.error("invalid 'encoding' argument")),
    };
    let is_latin1 = encoding == b"latin1";
    let is_utf8 = encoding == b"UTF-8";
    let skip_nul = lg(cx, 18);
    if quiet == NA_INTEGER {
        quiet = 0;
    }
    if blskip == NA_INTEGER {
        blskip = 1;
    }
    if multiline == NA_INTEGER {
        multiline = 1;
    }
    let nskip = nskip.max(0);
    let nlines = nlines.max(0);
    let nmax = nmax.max(0);
    let what = args.get(cx, 1);
    let stripwhite = args.get(cx, 11);
    if stripwhite.sexptype() != LGLSXP {
        return Err(cx.error("invalid 'strip.white' argument"));
    }
    let nstrip = stripwhite.len().unwrap_or(0);
    if nstrip != 1 && nstrip != what.len().unwrap_or(0) {
        return Err(cx.error("invalid 'strip.white' length"));
    }
    let lstrip: Vec<i32> = match stripwhite.view() {
        View::Lgl(v) => (0..v.len()).map(|i| v.get(i).0).collect(),
        _ => Vec::new(),
    };
    if args.get(cx, 8).sexptype() != STRSXP {
        return Err(cx.error("invalid 'na.strings' argument"));
    }
    let comstr = args.get(cx, 15);
    if comstr.sexptype() != STRSXP || comstr.len() != Some(1) {
        return Err(cx.error("invalid 'comment.char' argument"));
    }
    let sep = args.get(cx, 3);
    let sepchar: c_int = if matches!(sep.sexptype(), STRSXP | NILSXP) {
        if sep.len().unwrap_or(0) == 0 {
            0
        } else {
            let sc = first_native(cx, sep).unwrap_or_default();
            if strlen(&sc) > 1 {
                return Err(cx.error("invalid 'sep' value: must be one byte"));
            }
            sc.first().copied().unwrap_or(0) as c_int
        }
    } else {
        return Err(cx.error("invalid 'sep' argument"));
    };
    let dec = args.get(cx, 4);
    if matches!(dec.sexptype(), STRSXP | NILSXP) {
        if dec.len().unwrap_or(0) != 0 {
            let dc = first_native(cx, dec).unwrap_or_default();
            if strlen(&dc) != 1 {
                return Err(cx.error("invalid decimal separator: must be one byte"));
            }
        }
    } else {
        return Err(cx.error("invalid decimal separator"));
    }
    let quotes = args.get(cx, 5);
    let quoteset = match quotes.sexptype() {
        STRSXP if quotes.len().unwrap_or(0) > 0 => {
            let q = first_native(cx, quotes).unwrap_or_default();
            q[..strlen(&q)].to_vec()
        }
        // (STRING_ELT of an empty vector: robigo reads it as "")
        STRSXP | NILSXP => Vec::new(),
        _ => return Err(cx.error("invalid quote symbol set")),
    };
    let p = first_native(cx, comstr).unwrap_or_default();
    let comchar = match strlen(&p) {
        0 => None,
        1 => Some(p[0] as c_int),
        _ => return Err(cx.error("invalid 'comment.char' argument")),
    };
    if escapes == NA_INTEGER {
        return Err(cx.error("invalid 'allowEscapes' argument"));
    }
    if skip_nul == NA_INTEGER {
        return Err(cx.error("invalid 'skipNul' argument"));
    }
    // the columns, as scanVector and scanFrame take them: what robigo can
    // read is text
    let not_yet = |cx: &mut Cx, t: u8| {
        let ty = robigo_rtbuiltins::util::type2char(t);
        robigo_rtbuiltins::util::not_yet(cx, Some(call), &format!("scan() of a '{ty}' 'what'"))
    };
    let frame: Option<(Vec<Ty>, Option<Ref<'_>>)> = match what.sexptype() {
        STRSXP => None,
        VECSXP => {
            let v = what.as_vector::<Ref<'_>>().unwrap();
            if v.is_empty() {
                return Err(cx.error("empty 'what' specified"));
            }
            let mut cols = Vec::new();
            for i in 0..v.len() {
                let w = v.get(i);
                match w.sexptype() {
                    NILSXP => cols.push(Ty::Nil),
                    STRSXP => cols.push(Ty::Str),
                    t if robigo_rtbuiltins::attrib::is_vector(w) => return Err(not_yet(cx, t)),
                    _ => return Err(cx.error("invalid 'what' argument")),
                }
            }
            Some((cols, None))
        }
        t @ (robigo_heap::types::LGLSXP
        | robigo_heap::types::INTSXP
        | robigo_heap::types::REALSXP
        | robigo_heap::types::CPLXSXP
        | robigo_heap::types::RAWSXP) => return Err(not_yet(cx, t)),
        _ => return Err(cx.error("invalid 'what' argument")),
    };
    let frame_cols = frame.map(|(c, _)| c);
    let na_strings = strings(args.get(cx, 8));
    let names = if frame_cols.is_some() {
        robigo_rtbuiltins::attrib::names(what).map(strings)
    } else {
        None
    };

    let ii = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(0))?;
    with_conn(cx, ii, |_| ())?;
    if ii == 0 {
        return Err(robigo_rtbuiltins::util::not_yet(cx, Some(call), "scan() of the console"));
    }
    let wasopen = flags(cx, ii)?.isopen;
    if !wasopen {
        // UTF8out: a request; the mode "r" (kept, as strcpy keeps it)
        with_conn(cx, ii, |c| {
            c.c.utf8out = 1;
            c.set_mode("r");
        })?;
        if !super::open(cx, ii)? {
            return Err(cx.error("cannot open the connection"));
        }
        if !flags(cx, ii)?.canread {
            with_conn(cx, ii, close_conn)?;
            return Err(cx.error("cannot read from this connection"));
        }
    } else if !flags(cx, ii)?.canread {
        return Err(cx.error("cannot read from this connection"));
    }
    let ours = our_fgetc(cx);
    let utf8locale = cx.locale().utf8;
    let mut d = Local {
        na_strings: &na_strings,
        sepchar,
        quoteset,
        comchar,
        escapes: escapes != 0,
        save: 0,
        at_start: nskip == 0,
        embed_warn: false,
        skip_nul: skip_nul != 0,
        utf8locale,
        ours,
        warnings: Vec::new(),
    };
    let r = with_conn(cx, ii, |con| -> Result<(Read, bool), String> {
        for _ in 0..nskip {
            loop {
                let c = d.scanchar(false, con)?;
                if c == b'\n' as c_int || c == R_EOF {
                    break;
                }
            }
        }
        d.save = 0;
        let read = match &frame_cols {
            None => Read::Vector(scan_vector(
                &mut d,
                con,
                nmax,
                nlines,
                flush != 0,
                lstrip.first().is_some_and(|&s| s != 0),
                blskip != 0,
            )?),
            Some(cols) => {
                let (f, n) = scan_frame(
                &mut d,
                con,
                cols,
                nmax,
                nlines,
                flush != 0,
                fill != 0,
                &lstrip,
                blskip != 0,
                multiline != 0,
            )?;
                Read::Frame(f, n)
            }
        };
        Ok((read, con.c.utf8out != 0))
    })?;
    // the warnings as they arose, then the error; a connection scan opened
    // is closed (scan_cleanup)
    for w in std::mem::take(&mut d.warnings) {
        cx.warning(w)?;
    }
    let (read, utf8out) = match r {
        Ok(v) => v,
        Err(m) => {
            if !wasopen {
                with_conn(cx, ii, close_conn)?;
            }
            return Err(cx.error(&m));
        }
    };
    if quiet == 0 {
        let msg = match &read {
            Read::Vector(v) => format!("Read {} item{}\n", v.len(), if v.len() == 1 { "" } else { "s" }),
            &Read::Frame(_, n) => {
                format!("Read {n} record{}\n", if n == 1 { "" } else { "s" })
            }
        };
        cx.reprintf(msg.as_bytes());
    }
    // an unscanchar-ed character goes back on a connection left open
    if d.save != 0 && wasopen {
        let line = [d.save as u8];
        let r = with_conn(cx, ii, |c| c.push_back_line(&line, false))?;
        r.map_err(|m| cx.error(&m))?;
    }
    if !wasopen {
        with_conn(cx, ii, close_conn)?;
    }
    if skip_nul == 0 && d.embed_warn {
        cx.warning("embedded nul(s) found in input")?;
    }
    // insertString's mark
    let enc = if utf8out || is_utf8 {
        Encoding::Utf8
    } else if is_latin1 {
        Encoding::Latin1
    } else {
        Encoding::Native
    };
    cx.build(out, |cx| {
        match &read {
            Read::Vector(items) => strvec(cx, items, enc),
            Read::Frame(cols, _) => {
                let mut l = cx.alloc_vector::<Ref<'_>>(cols.len())?;
                for (i, c) in cols.iter().enumerate() {
                    match c {
                        Some(items) => l.set(i, strvec(cx, items, enc)?),
                        None => l.set(i, cx.nil()),
                    }
                }
                if let Some(nm) = &names {
                    let mut n = cx.alloc_vector::<Char<'_>>(nm.len())?;
                    for (i, s) in nm.iter().enumerate() {
                        n.set(i, cx.mk_char(s, Encoding::Native)?);
                    }
                    let mut a = robigo_rtbuiltins::attrib::Attrs::new();
                    a.install(robigo_rtbuiltins::attrib::Attr {
                        tag: cx.sym(robigo_rtcore::RSym::Names),
                        value: n.finish().into(),
                    });
                    a.apply(cx, &mut l)?;
                }
                Ok(l.finish().into())
            }
        }
    })
}

/// A character vector of `items` (None: NA), marked `enc`.
fn strvec<'g>(cx: &'g Cx, items: &[Option<Vec<u8>>], enc: Encoding) -> Result<Ref<'g>, robigo_rtcore::NeedGc> {
    let mut v = cx.alloc_vector::<Char<'g>>(items.len())?;
    for (i, it) in items.iter().enumerate() {
        v.set(
            i,
            match it {
                Some(b) => cx.mk_char(b, enc)?,
                None => cx.na_string(),
            },
        );
    }
    Ok(v.finish().into())
}
