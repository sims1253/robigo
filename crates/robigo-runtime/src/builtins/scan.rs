//! `scan()` (port of GNU R's src/main/scan.c: do_scan, scanVector,
//! scanFrame, fillBuffer, scanchar, extractItem).

use super::{arg, internal};
use crate::coerce::{as_integer, as_logical, is_blank, r_strtod5};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::value::*;

pub static DEFS: &[Builtin] = &[internal("scan", do_scan, 19)];

pub(crate) const EOF: i32 = -1;
pub(crate) const NO_COMCHAR: i32 = 100000;

/// The reader state shared with utils' io.c ports (R's `LocalData`).
pub(crate) struct Scan {
    pub con: i32,
    pub na_strings: Vec<RStr>,
    pub quiet: bool,
    pub sepchar: i32,
    pub decchar: u8,
    pub quoteset: Vec<u8>,
    pub comchar: i32,
    pub ttyflag: bool,
    pub escapes: bool,
    pub save: i32,
    pub at_start: bool,
    pub embed_warn: bool,
    pub skip_nul: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Ty {
    Null,
    Lgl,
    Int,
    Real,
    Cplx,
    Str,
    Raw,
}

fn ty_of(v: &Value) -> Option<Ty> {
    Some(match v {
        Value::Null => Ty::Null,
        Value::Lgl(_) => Ty::Lgl,
        Value::Int(_) => Ty::Int,
        Value::Real(_) => Ty::Real,
        Value::Cplx(_) => Ty::Cplx,
        Value::Str(_) => Ty::Str,
        Value::Raw(_) => Ty::Raw,
        _ => return None,
    })
}

/// A growing column of scanned values.
enum Col {
    Null,
    Lgl(Vec<Lgl>),
    Int(Vec<i32>),
    Real(Vec<f64>),
    Cplx(Vec<Complex>),
    Str(Vec<RStr>),
    Raw(Vec<u8>),
}

impl Col {
    fn new(t: Ty) -> Col {
        match t {
            Ty::Null => Col::Null,
            Ty::Lgl => Col::Lgl(vec![]),
            Ty::Int => Col::Int(vec![]),
            Ty::Real => Col::Real(vec![]),
            Ty::Cplx => Col::Cplx(vec![]),
            Ty::Str => Col::Str(vec![]),
            Ty::Raw => Col::Raw(vec![]),
        }
    }

    fn ty(&self) -> Ty {
        match self {
            Col::Null => Ty::Null,
            Col::Lgl(_) => Ty::Lgl,
            Col::Int(_) => Ty::Int,
            Col::Real(_) => Ty::Real,
            Col::Cplx(_) => Ty::Cplx,
            Col::Str(_) => Ty::Str,
            Col::Raw(_) => Ty::Raw,
        }
    }

    fn into_value(self) -> Value {
        match self {
            Col::Null => Value::Null,
            Col::Lgl(v) => Value::lgls(v),
            Col::Int(v) => Value::ints(v),
            Col::Real(v) => Value::reals(v),
            Col::Cplx(v) => Value::Cplx(std::rc::Rc::new(Vector::new(v))),
            Col::Str(v) => Value::strs(v),
            Col::Raw(v) => Value::raws(v),
        }
    }
}

impl Scan {
    /// The defaults of utils' io.c (`LocalData data = {...}`).
    pub(crate) fn new(con: i32) -> Scan {
        Scan {
            con,
            na_strings: vec![],
            quiet: false,
            sepchar: 0,
            decchar: b'.',
            quoteset: vec![],
            comchar: NO_COMCHAR,
            ttyflag: con == 0,
            escapes: false,
            save: 0,
            at_start: false,
            embed_warn: false,
            skip_nul: false,
        }
    }

    // R: scan.c:scanchar_raw
    fn raw(&mut self, i: &mut Interp) -> RResult<i32> {
        let mut c = super::io::conn_getc(i, self.con)?;
        if c == 0 {
            if self.skip_nul {
                while c == 0 {
                    c = super::io::conn_getc(i, self.con)?;
                }
            } else {
                self.embed_warn = true;
            }
        }
        Ok(c)
    }

    pub(crate) fn unscanchar(&mut self, c: i32) {
        self.save = c;
    }

    // R: scan.c:scanchar
    pub(crate) fn scanchar(&mut self, i: &mut Interp, in_quote: bool) -> RResult<i32> {
        let mut next = if self.save != 0 {
            let c = self.save;
            self.save = 0;
            c
        } else {
            self.raw(i)?
        };
        if next == self.comchar && !in_quote {
            loop {
                next = self.raw(i)?;
                if next == '\n' as i32 || next == EOF {
                    break;
                }
            }
        }
        if next == '\\' as i32 && self.escapes {
            next = self.raw(i)?;
            if ('0' as i32..='8' as i32).contains(&next) {
                let mut octal = next - '0' as i32;
                next = self.raw(i)?;
                if ('0' as i32..='8' as i32).contains(&next) {
                    octal = 8 * octal + next - '0' as i32;
                    next = self.raw(i)?;
                    if ('0' as i32..='8' as i32).contains(&next) {
                        octal = 8 * octal + next - '0' as i32;
                    } else {
                        self.unscanchar(next);
                    }
                } else {
                    self.unscanchar(next);
                }
                next = octal;
            } else {
                next = match next as u8 as char {
                    'a' if next < 128 => 7,
                    'b' if next < 128 => 8,
                    'f' if next < 128 => 12,
                    'n' if next < 128 => '\n' as i32,
                    'r' if next < 128 => '\r' as i32,
                    't' if next < 128 => '\t' as i32,
                    'v' if next < 128 => 11,
                    'x' if next < 128 => {
                        let mut val = 0;
                        for _ in 0..2 {
                            let c = self.raw(i)?;
                            let ext = match c as u8 {
                                b'0'..=b'9' if c < 128 => c - '0' as i32,
                                b'A'..=b'F' if c < 128 => c - 'A' as i32 + 10,
                                b'a'..=b'f' if c < 128 => c - 'a' as i32 + 10,
                                _ => {
                                    self.unscanchar(c);
                                    break;
                                }
                            };
                            val = 16 * val + ext;
                        }
                        val
                    }
                    _ => {
                        if in_quote && next >= 0 && self.quoteset.contains(&(next as u8)) {
                            self.unscanchar(next);
                            '\\' as i32
                        } else {
                            next
                        }
                    }
                };
            }
        }
        Ok(next)
    }

    pub(crate) fn is_quote(&self, c: i32) -> bool {
        c > 0 && c < 256 && self.quoteset.contains(&(c as u8))
    }

    /// Read one field into a buffer; returns it and `bch`, the character
    /// that ended it (separator, newline, CR or EOF).
    // R: scan.c:fillBuffer
    fn fill_buffer(&mut self, i: &mut Interp, ty: Ty, mut strip: bool) -> RResult<(Vec<u8>, i32)> {
        let rspace = |c: i32| c == ' ' as i32 || c == '\t' as i32 || c == '\n' as i32 || c == '\r' as i32;
        let mut buf: Vec<u8> = Vec::new();
        let mut filled = 1;
        let mut mm = 0;
        let quoted = ty == Ty::Str || ty == Ty::Null;
        'done: {
            if self.sepchar == 0 {
                strip = false;
                let mut c;
                loop {
                    c = self.scanchar(i, false)?;
                    if c != ' ' as i32 && c != '\t' as i32 {
                        break;
                    }
                }
                if c == '\n' as i32 || c == '\r' as i32 || c == EOF {
                    filled = c;
                    break 'done;
                }
                if quoted && self.is_quote(c) {
                    let quote = c;
                    loop {
                        c = self.scanchar(i, true)?;
                        if c == EOF || c == quote {
                            break;
                        }
                        if c == '\\' as i32 {
                            c = self.scanchar(i, true)?;
                            if c == EOF {
                                break;
                            }
                            if c != quote {
                                buf.push(b'\\');
                            }
                        }
                        buf.push(c as u8);
                    }
                    if c == EOF {
                        i.warning("EOF within quoted string")?;
                    }
                    c = self.scanchar(i, false)?;
                    mm = buf.len();
                } else {
                    loop {
                        buf.push(c as u8);
                        c = self.scanchar(i, false)?;
                        if rspace(c) || c == EOF {
                            break;
                        }
                    }
                }
                while c == ' ' as i32 || c == '\t' as i32 {
                    c = self.scanchar(i, false)?;
                }
                if c == '\n' as i32 || c == '\r' as i32 || c == EOF {
                    filled = c;
                } else {
                    self.unscanchar(c);
                }
            } else {
                let mut c;
                loop {
                    c = self.scanchar(i, false)?;
                    if c == self.sepchar || c == '\n' as i32 || c == '\r' as i32 || c == EOF {
                        break;
                    }
                    if ty != Ty::Str {
                        while c == ' ' as i32 || c == '\t' as i32 {
                            c = self.scanchar(i, false)?;
                            if c == self.sepchar || c == '\n' as i32 || c == '\r' as i32 || c == EOF {
                                filled = c;
                                break 'done;
                            }
                        }
                    }
                    if quoted && c != 0 && self.is_quote(c) {
                        let quote = c;
                        loop {
                            loop {
                                c = self.scanchar(i, true)?;
                                if c == EOF || c == quote {
                                    break;
                                }
                                buf.push(c as u8);
                            }
                            if c == EOF {
                                i.warning("EOF within quoted string")?;
                            }
                            c = self.scanchar(i, true)?;
                            if c == quote {
                                buf.push(quote as u8);
                                continue;
                            }
                            break;
                        }
                        mm = buf.len();
                        if c == self.sepchar || c == '\n' as i32 || c == '\r' as i32 || c == EOF {
                            filled = c;
                            break 'done;
                        }
                        self.unscanchar(c);
                        continue;
                    }
                    if !strip || !buf.is_empty() || !rspace(c) {
                        buf.push(c as u8);
                    }
                }
                filled = c;
            }
        }
        if strip && buf.len() > mm {
            while buf.len() > mm && rspace(*buf.last().unwrap() as i32) {
                buf.pop();
            }
        }
        if self.at_start && buf.starts_with(b"\xef\xbb\xbf") {
            buf.drain(..3);
        }
        self.at_start = false;
        Ok((buf, filled))
    }

    // R: scan.c:isNAstring
    pub(crate) fn is_na_string(&self, buf: &str, mode: i32) -> bool {
        if mode == 0 && buf.is_empty() {
            return true;
        }
        self.na_strings.iter().any(|s| s.as_deref() == Some(buf))
    }

    // R: scan.c:expected
    fn expected(&mut self, i: &mut Interp, what: &str, got: &str) -> Flow {
        if self.ttyflag {
            while let Ok(c) = self.scanchar(i, false) {
                if c == EOF || c == '\n' as i32 {
                    break;
                }
            }
        }
        Flow::error(format!("scan() expected '{}', got '{}'", what, got))
    }

    // R: scan.c:extractItem
    fn extract(&mut self, i: &mut Interp, buf: &[u8], col: &mut Col) -> RResult<()> {
        let s = String::from_utf8_lossy(buf).into_owned();
        match col {
            Col::Null => {}
            Col::Lgl(v) => {
                if self.is_na_string(&s, 0) {
                    v.push(NA_LOGICAL);
                } else {
                    let tr = matches!(s.as_str(), "T" | "True" | "TRUE" | "true");
                    let fa = matches!(s.as_str(), "F" | "False" | "FALSE" | "false");
                    if tr || fa {
                        v.push(tr as i32);
                    } else {
                        return Err(self.expected(i, "a logical", &s));
                    }
                }
            }
            Col::Int(v) => {
                if self.is_na_string(&s, 0) {
                    v.push(NA_INTEGER);
                } else {
                    let x = strtoi(&s);
                    if x == NA_INTEGER {
                        return Err(self.expected(i, "an integer", &s));
                    }
                    v.push(x);
                }
            }
            Col::Real(v) => {
                if self.is_na_string(&s, 0) {
                    v.push(NA_REAL);
                } else {
                    let (x, used) = r_strtod5(buf, self.decchar, false, 0);
                    if !is_blank(&s[used.min(s.len())..]) {
                        return Err(self.expected(i, "a real", &s));
                    }
                    v.push(x);
                }
            }
            Col::Cplx(v) => {
                if self.is_na_string(&s, 0) {
                    v.push(Complex::NA);
                } else {
                    match strtoc(buf, self.decchar) {
                        Some((z, used)) if is_blank(&s[used.min(s.len())..]) => v.push(z),
                        _ => return Err(self.expected(i, "a complex", &s)),
                    }
                }
            }
            Col::Str(v) => {
                if self.is_na_string(&s, 1) {
                    v.push(None);
                } else {
                    v.push(rstr(&s));
                }
            }
            Col::Raw(v) => {
                if self.is_na_string(&s, 0) {
                    v.push(0);
                } else {
                    let (r, used) = strtoraw(buf);
                    if !is_blank(&s[used.min(s.len())..]) {
                        return Err(self.expected(i, "a raw", &s));
                    }
                    v.push(r);
                }
            }
        }
        Ok(())
    }
}

/// `strtol(s, 10)` with R's range and trailing-garbage checks.
// R: scan.c:Strtoi
pub(crate) fn strtoi(s: &str) -> i32 {
    let t = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (neg, body) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    if body.is_empty() || !body.bytes().all(|b| b.is_ascii_digit()) {
        return NA_INTEGER;
    }
    match body.parse::<i64>() {
        Ok(v) => {
            let v = if neg { -v } else { v };
            if v > i32::MAX as i64 || v < i32::MIN as i64 { NA_INTEGER } else { v as i32 }
        }
        Err(_) => NA_INTEGER,
    }
}

// R: scan.c:strtoc
fn strtoc(s: &[u8], dec: u8) -> Option<(Complex, usize)> {
    let (x, endp) = r_strtod5(s, dec, false, 0);
    let rest = &s[endp..];
    if rest.iter().all(|c| c.is_ascii_whitespace()) {
        return Some((Complex::new(x, 0.0), endp));
    }
    if rest.first() == Some(&b'i') {
        return Some((Complex::new(0.0, x), endp + 1));
    }
    let (y, e2) = r_strtod5(rest, dec, false, 0);
    if rest.get(e2) == Some(&b'i') {
        return Some((Complex::new(x, y), endp + e2 + 1));
    }
    None
}

// R: scan.c:strtoraw
fn strtoraw(s: &[u8]) -> (u8, usize) {
    let mut p = 0;
    while matches!(s.get(p), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        p += 1;
    }
    let mut val = 0u32;
    for _ in 0..2 {
        let c = s.get(p).copied().unwrap_or(0);
        let d = match c {
            b'0'..=b'9' => c - b'0',
            b'A'..=b'F' => c - b'A' + 10,
            b'a'..=b'f' => c - b'a' + 10,
            _ => {
                val = 0;
                break;
            }
        };
        val = val * 16 + d as u32;
        p += 1;
    }
    (val as u8, p)
}

fn as_xlen(v: &Value) -> i64 {
    let x = crate::coerce::as_real(v);
    if x.is_nan() { -1 } else { x as i64 }
}

// R: scan.c:scanVector
fn scan_vector(i: &mut Interp, d: &mut Scan, ty: Ty, maxitems: i64, maxlines: i64, flush: bool, strip: bool, blskip: bool) -> RResult<Value> {
    let mut col = Col::new(ty);
    let (mut n, mut nprev, mut linesread): (i64, i64, i64) = (0, 0, 0);
    let mut bch = 1;
    loop {
        if bch == EOF {
            break;
        } else if bch == '\n' as i32 {
            linesread += 1;
            if linesread == maxlines {
                break;
            }
            nprev = n;
        }
        let (buf, b) = d.fill_buffer(i, ty, strip)?;
        bch = b;
        if nprev == n && buf.is_empty() && ((blskip && bch == '\n' as i32) || bch == EOF) {
            if d.ttyflag || bch == EOF {
                break;
            }
        } else {
            d.extract(i, &buf, &mut col)?;
            n += 1;
            if n == maxitems {
                if d.ttyflag && bch != '\n' as i32 {
                    while d.scanchar(i, false)? != '\n' as i32 {}
                }
                break;
            }
        }
        if flush && bch != '\n' as i32 && bch != EOF {
            let mut c;
            loop {
                c = d.scanchar(i, false)?;
                if c == '\n' as i32 || c == EOF {
                    break;
                }
            }
            bch = c;
        }
    }
    if !d.quiet {
        i.write_err(&format!("Read {} item{}\n", n, if n == 1 { "" } else { "s" }));
    }
    Ok(col.into_value())
}

// R: scan.c:scanFrame
#[allow(clippy::too_many_arguments)]
fn scan_frame(
    i: &mut Interp,
    d: &mut Scan,
    what: &Value,
    maxitems: i64,
    maxlines: i64,
    flush: bool,
    fill: bool,
    stripwhite: &[Lgl],
    blskip: bool,
    multiline: bool,
) -> RResult<Value> {
    let Value::List(w) = what else { unreachable!() };
    let nc = w.data.len();
    if nc == 0 {
        return Err(Flow::error("empty 'what' specified"));
    }
    let mut cols: Vec<Col> = Vec::with_capacity(nc);
    for x in &w.data {
        match ty_of(x) {
            Some(t) => cols.push(Col::new(t)),
            None => return Err(Flow::error("invalid 'what' argument")),
        }
    }
    let vec_strip = stripwhite.len() == nc;
    let mut strip = stripwhite[0] != 0;
    let (mut n, mut linesread, mut colsread, mut ii, mut badline): (i64, i64, usize, usize, i64) = (0, 0, 0, 0, 0);
    let mut bch = 1;
    'done: loop {
        if bch == EOF {
            break 'done;
        } else if bch == '\n' as i32 {
            linesread += 1;
            if colsread != 0 {
                if fill {
                    for col in cols.iter_mut().skip(colsread) {
                        d.extract(i, b"", col)?;
                    }
                    n += 1;
                    ii = 0;
                    colsread = 0;
                } else if badline == 0 && !multiline {
                    badline = linesread;
                }
                if badline != 0 && !multiline {
                    return Err(Flow::error(format!("line {} did not have {} elements", badline, nc)));
                }
            }
            if maxitems > 0 && n >= maxitems {
                break 'done;
            }
            if maxlines > 0 && linesread == maxlines {
                break 'done;
            }
        }
        if vec_strip {
            strip = stripwhite[colsread] != 0;
        }
        let ty = cols[ii].ty();
        let (buf, b) = d.fill_buffer(i, ty, strip)?;
        bch = b;
        if colsread == 0 && buf.is_empty() && ((blskip && bch == '\n' as i32) || bch == EOF) {
            if d.ttyflag || bch == EOF {
                break;
            }
        } else {
            d.extract(i, &buf, &mut cols[ii])?;
            ii += 1;
            colsread += 1;
            if colsread == nc {
                n += 1;
                ii = 0;
                colsread = 0;
                if flush && bch != '\n' as i32 && bch != EOF {
                    let mut c;
                    loop {
                        c = d.scanchar(i, false)?;
                        if c == '\n' as i32 || c == EOF {
                            break;
                        }
                    }
                    bch = c;
                }
            }
        }
    }
    if colsread != 0 {
        if !fill {
            i.warning("number of items read is not a multiple of the number of columns")?;
        }
        for col in cols.iter_mut().skip(colsread) {
            d.extract(i, b"", col)?;
        }
        n += 1;
    }
    if !d.quiet {
        i.write_err(&format!("Read {} record{}\n", n, if n == 1 { "" } else { "s" }));
    }
    let mut ans = Value::list(cols.into_iter().map(Col::into_value).collect());
    if let Some(names) = what.names() {
        ans.set_attr_raw(crate::sym::s::NAMES, names);
    }
    Ok(ans)
}

// R: scan.c:do_scan
fn do_scan(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let file = arg(&args, 0);
    let what = arg(&args, 1);
    let nmax = as_xlen(&arg(&args, 2)).max(0);
    let sep = arg(&args, 3);
    let dec = arg(&args, 4);
    let quotes = arg(&args, 5);
    let nskip = as_xlen(&arg(&args, 6)).max(0);
    let nlines = as_xlen(&arg(&args, 7)).max(0);
    let na_strings = arg(&args, 8);
    let flush = as_logical(&arg(&args, 9)) == TRUE;
    let fill = as_logical(&arg(&args, 10)) == TRUE;
    let stripwhite = arg(&args, 11);
    let quiet = as_logical(&arg(&args, 12));
    let blskip = as_logical(&arg(&args, 13));
    let multiline = as_logical(&arg(&args, 14));
    let comstr = arg(&args, 15);
    let escapes = as_logical(&arg(&args, 16));
    let Value::Str(enc) = arg(&args, 17) else { return Err(Flow::error("invalid 'encoding' argument")) };
    if enc.data.len() != 1 {
        return Err(Flow::error("invalid 'encoding' argument"));
    }
    let skip_nul = as_logical(&arg(&args, 18));
    let Value::Lgl(sw) = &stripwhite else { return Err(Flow::error("invalid 'strip.white' argument")) };
    if sw.data.len() != 1 && sw.data.len() != what.len() {
        return Err(Flow::error("invalid 'strip.white' length"));
    }
    let Value::Str(na) = &na_strings else { return Err(Flow::error("invalid 'na.strings' argument")) };
    let comchar = match &comstr {
        Value::Str(c) if c.data.len() == 1 => {
            let p = c.data[0].as_deref().unwrap_or("NA").as_bytes();
            if p.len() > 1 {
                return Err(Flow::error("invalid 'comment.char' argument"));
            }
            p.first().map(|&b| b as i32).unwrap_or(NO_COMCHAR)
        }
        _ => return Err(Flow::error("invalid 'comment.char' argument")),
    };
    let sepchar = match &sep {
        Value::Null => 0,
        Value::Str(s) if s.data.is_empty() => 0,
        Value::Str(s) => {
            let sc = s.data[0].as_deref().unwrap_or("NA").as_bytes();
            if sc.len() > 1 {
                return Err(Flow::error("invalid 'sep' value: must be one byte"));
            }
            sc.first().map(|&b| b as i32).unwrap_or(0)
        }
        _ => return Err(Flow::error("invalid 'sep' argument")),
    };
    let decchar = match &dec {
        Value::Null => b'.',
        Value::Str(s) if s.data.is_empty() => b'.',
        Value::Str(s) => {
            let dc = s.data[0].as_deref().unwrap_or("NA").as_bytes();
            if dc.len() != 1 {
                return Err(Flow::error("invalid decimal separator: must be one byte"));
            }
            dc[0]
        }
        _ => return Err(Flow::error("invalid decimal separator")),
    };
    let quoteset = match &quotes {
        Value::Str(q) => q.data.first().and_then(|x| x.as_deref()).unwrap_or("").as_bytes().to_vec(),
        Value::Null => vec![],
        _ => return Err(Flow::error("invalid quote symbol set")),
    };
    if escapes == NA_LOGICAL {
        return Err(Flow::error("invalid 'allowEscapes' argument"));
    }
    if skip_nul == NA_LOGICAL {
        return Err(Flow::error("invalid 'skipNul' argument"));
    }
    let con = as_integer(&file);
    let mut d = Scan {
        con,
        na_strings: na.data.clone(),
        quiet: quiet != NA_LOGICAL && quiet != 0,
        sepchar,
        decchar,
        quoteset,
        comchar,
        ttyflag: con == 0,
        escapes: escapes != 0,
        save: 0,
        at_start: false,
        embed_warn: false,
        skip_nul: skip_nul != 0,
    };
    let mut wasopen = true;
    if con != 0 {
        d.at_start = nskip == 0;
        let (isopen, ..) = super::io::conn_state(i, con)?;
        wasopen = isopen;
        if !wasopen {
            if !super::io::open_in_mode(i, con, "r")? {
                return Err(Flow::error("cannot open the connection"));
            }
            if !super::io::conn_state(i, con)?.1 {
                super::io::close_conn(i, con)?;
                return Err(Flow::error("cannot read from this connection"));
            }
        } else if !super::io::conn_state(i, con)?.1 {
            return Err(Flow::error("cannot read from this connection"));
        }
        for _ in 0..nskip {
            loop {
                let c = d.scanchar(i, false)?;
                if c == '\n' as i32 || c == EOF {
                    break;
                }
            }
        }
    }
    d.save = 0;
    let blskip = blskip == NA_LOGICAL || blskip != 0;
    let multiline = multiline == NA_LOGICAL || multiline != 0;
    let strip = sw.data[0] != 0 && sw.data[0] != NA_LOGICAL;
    let res = match ty_of(&what) {
        Some(t) if t != Ty::Null => scan_vector(i, &mut d, t, nmax, nlines, flush, strip, blskip),
        _ if matches!(what, Value::List(_)) => {
            scan_frame(i, &mut d, &what, nmax, nlines, flush, fill, &sw.data, blskip, multiline)
        }
        _ => Err(Flow::error("invalid 'what' argument")),
    };
    if res.is_ok() && d.save != 0 && !d.ttyflag && wasopen {
        super::io::conn_pushback(i, con, &[d.save as u8])?;
    }
    if !d.ttyflag && !wasopen {
        super::io::close_conn(i, con)?;
    }
    let ans = res?;
    if !d.skip_nul && d.embed_warn {
        i.warning("embedded nul(s) found in input")?;
    }
    Ok(ans)
}
