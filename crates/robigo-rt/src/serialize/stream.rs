//! serialize.c's streams: the three formats' encodings of integers, reals,
//! bytes and strings (`OutInteger`, `OutReal`, `OutString`, `InInteger`,
//! `InReal`, `InString`, `InWord`, ...), over an output buffer and an
//! input that is memory or a connection read on demand.
//!
//! The input keeps what it has read while a look-ahead is open
//! ([`InStream::mark`]): the reader peeks at an environment's contents
//! before it makes the environment (`read.rs`), and rewinds.

use super::Format;
use robigo_rtcore::Cx;

/// A read failure: GNU R's message (`error(_(...))`).
pub type InErr = String;
pub type InResult<T> = Result<T, InErr>;

const NA_INTEGER: i32 = i32::MIN;

/// The output: everything is buffered, and the caller hands the bytes to
/// their sink (a raw vector, a connection) at the end.
pub struct OutStream {
    pub fmt: Format,
    pub version: i32,
    pub buf: Vec<u8>,
}

/// `%.16g` as C's printf writes it.
pub fn fmt_g(d: f64, prec: usize) -> String {
    if d == 0.0 {
        return if d.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    let p = prec.max(1);
    let e = format!("{:.*e}", p - 1, d);
    let (mant, exp) = e.split_once('e').unwrap();
    let x: i32 = exp.parse().unwrap();
    if x < -4 || x >= p as i32 {
        let mant = strip_zeros(mant);
        let sign = if x < 0 { '-' } else { '+' };
        format!("{mant}e{sign}{:02}", x.abs())
    } else {
        let decimals = (p as i32 - 1 - x).max(0) as usize;
        strip_zeros(&format!("{:.*}", decimals, d)).to_string()
    }
}

fn strip_zeros(s: &str) -> &str {
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.') } else { s }
}

/// `%a` as glibc's printf writes it.
pub fn fmt_a(d: f64) -> String {
    let bits = d.to_bits();
    let sign = if bits >> 63 != 0 { "-" } else { "" };
    let exp = ((bits >> 52) & 0x7ff) as i64;
    let frac = bits & ((1u64 << 52) - 1);
    if exp == 0 && frac == 0 {
        return format!("{sign}0x0p+0");
    }
    let (lead, e) = if exp == 0 { (0, -1022) } else { (1, exp - 1023) };
    let mut hex = format!("{frac:013x}");
    while hex.ends_with('0') {
        hex.pop();
    }
    let es = if e < 0 { format!("-{}", -e) } else { format!("+{e}") };
    if hex.is_empty() { format!("{sign}0x{lead}p{es}") } else { format!("{sign}0x{lead}.{hex}p{es}") }
}

impl OutStream {
    pub fn new(fmt: Format, version: i32) -> OutStream {
        OutStream { fmt, version, buf: Vec::with_capacity(256) }
    }

    #[inline]
    pub fn bytes(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }

    // R: serialize.c:OutInteger
    #[inline]
    pub fn integer(&mut self, i: i32) {
        match self.fmt {
            Format::Ascii | Format::AsciiHex => {
                if i == NA_INTEGER {
                    self.bytes(b"NA\n");
                } else {
                    self.bytes(format!("{i}\n").as_bytes());
                }
            }
            Format::Binary => self.bytes(&i.to_ne_bytes()),
            _ => self.bytes(&i.to_be_bytes()),
        }
    }

    fn ascii_nonfinite(&mut self, d: f64) -> bool {
        if d.is_finite() {
            return false;
        }
        if robigo_text::is_na(d) {
            self.bytes(b"NA\n");
        } else if d.is_nan() {
            self.bytes(b"NaN\n");
        } else if d < 0.0 {
            self.bytes(b"-Inf\n");
        } else {
            self.bytes(b"Inf\n");
        }
        true
    }

    // R: serialize.c:OutReal
    #[inline]
    pub fn real(&mut self, d: f64) {
        match self.fmt {
            Format::Ascii => {
                if !self.ascii_nonfinite(d) {
                    let s = fmt_g(d, 16);
                    self.bytes(s.as_bytes());
                    self.bytes(b"\n");
                }
            }
            Format::AsciiHex => {
                if !self.ascii_nonfinite(d) {
                    let s = fmt_a(d);
                    self.bytes(s.as_bytes());
                    self.bytes(b"\n");
                }
            }
            Format::Binary => self.bytes(&d.to_ne_bytes()),
            _ => self.bytes(&d.to_be_bytes()),
        }
    }

    // R: serialize.c:OutByte
    #[inline]
    pub fn byte(&mut self, b: u8) {
        match self.fmt {
            Format::Ascii | Format::AsciiHex => self.bytes(format!("{b:02x}\n").as_bytes()),
            _ => self.buf.push(b),
        }
    }

    // R: serialize.c:OutString
    pub fn string(&mut self, s: &[u8]) {
        if matches!(self.fmt, Format::Ascii | Format::AsciiHex) {
            for &c in s {
                match c {
                    b'\n' => self.bytes(b"\\n"),
                    b'\t' => self.bytes(b"\\t"),
                    0x0b => self.bytes(b"\\v"),
                    0x08 => self.bytes(b"\\b"),
                    b'\r' => self.bytes(b"\\r"),
                    0x0c => self.bytes(b"\\f"),
                    0x07 => self.bytes(b"\\a"),
                    b'\\' => self.bytes(b"\\\\"),
                    b'?' => self.bytes(b"\\?"),
                    b'\'' => self.bytes(b"\\'"),
                    b'"' => self.bytes(b"\\\""),
                    // (a signed char: bytes past 126 are negative, so <= 32)
                    _ if c <= 32 || c > 126 => self.bytes(format!("\\{c:03o}").as_bytes()),
                    _ => self.buf.push(c),
                }
            }
            self.buf.push(b'\n');
        } else {
            self.bytes(s);
        }
    }

    pub fn ints(&mut self, xs: &[i32]) {
        match self.fmt {
            Format::Binary => {
                self.buf.reserve(xs.len() * 4);
                for &x in xs {
                    self.buf.extend_from_slice(&x.to_ne_bytes());
                }
            }
            Format::Xdr => {
                self.buf.reserve(xs.len() * 4);
                for &x in xs {
                    self.buf.extend_from_slice(&x.to_be_bytes());
                }
            }
            _ => {
                for &x in xs {
                    self.integer(x);
                }
            }
        }
    }

    pub fn reals(&mut self, xs: &[f64]) {
        match self.fmt {
            Format::Binary => {
                self.buf.reserve(xs.len() * 8);
                for &x in xs {
                    self.buf.extend_from_slice(&x.to_ne_bytes());
                }
            }
            Format::Xdr => {
                self.buf.reserve(xs.len() * 8);
                for &x in xs {
                    self.buf.extend_from_slice(&x.to_be_bytes());
                }
            }
            _ => {
                for &x in xs {
                    self.real(x);
                }
            }
        }
    }
}

/// Where the input comes from.
#[derive(Clone, Copy, Debug)]
pub enum Source {
    /// Everything is in the buffer.
    Memory,
    /// A connection, read as needed: its number, and whether it is in text
    /// mode (`con->text`: `Rconn_fgetc` per byte, here a read of each).
    Conn(i32, bool),
}

pub struct InStream {
    pub fmt: Format,
    data: Vec<u8>,
    pos: usize,
    src: Source,
    marks: usize,
    /// What the connection's read failed with (a message of the
    /// connection layer, given instead of the stream's own).
    pub conn_error: Option<String>,
}

#[inline]
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

impl InStream {
    pub fn memory(data: Vec<u8>) -> InStream {
        InStream { fmt: Format::Any, data, pos: 0, src: Source::Memory, marks: 0, conn_error: None }
    }

    pub fn conn(n: i32, text: bool) -> InStream {
        InStream { fmt: Format::Any, data: Vec::new(), pos: 0, src: Source::Conn(n, text), marks: 0, conn_error: None }
    }

    /// Make `n` more bytes available from `pos` on.
    fn fill(&mut self, cx: &Cx, n: usize) -> InResult<()> {
        if self.pos + n <= self.data.len() {
            return Ok(());
        }
        match self.src {
            Source::Memory => Err("read error".into()),
            Source::Conn(c, _) => {
                if self.marks == 0 && self.pos > 0 {
                    self.data.drain(..self.pos);
                    self.pos = 0;
                }
                let need = self.pos + n - self.data.len();
                let start = self.data.len();
                self.data.resize(start + need, 0);
                let r = super::conn_read_raw(cx, c, &mut self.data[start..]);
                match r {
                    Ok(()) => Ok(()),
                    Err(m) => {
                        self.data.truncate(start);
                        self.conn_error = Some(m.clone());
                        Err(m)
                    }
                }
            }
        }
    }

    /// Start a look-ahead: [`InStream::reset`] returns to here.
    pub fn mark(&mut self) -> usize {
        self.marks += 1;
        self.pos
    }

    pub fn reset(&mut self, at: usize) {
        self.marks -= 1;
        self.pos = at;
    }

    /// `stream->InBytes(stream, buf, n)`.
    #[inline]
    pub fn in_bytes(&mut self, cx: &Cx, n: usize) -> InResult<&[u8]> {
        self.fill(cx, n)?;
        let p = self.pos;
        self.pos += n;
        Ok(&self.data[p..p + n])
    }

    /// `stream->InChar(stream)`: a byte, or None at the end of a memory
    /// stream for GNU R's EOF.
    #[inline]
    pub fn in_char(&mut self, cx: &Cx) -> InResult<Option<u8>> {
        match self.fill(cx, 1) {
            Ok(()) => {
                self.pos += 1;
                Ok(Some(self.data[self.pos - 1]))
            }
            Err(e) => match self.src {
                // InCharMem: "read error" past the end
                Source::Memory => Err(e),
                Source::Conn(..) => Err(e),
            },
        }
    }

    // R: serialize.c:InWord
    fn in_word(&mut self, cx: &Cx) -> InResult<Vec<u8>> {
        let mut c;
        loop {
            c = self.in_char(cx)?.ok_or("read error")?;
            if !is_space(c) {
                break;
            }
        }
        let mut w = Vec::new();
        while !is_space(c) && w.len() < 128 {
            w.push(c);
            c = match self.in_char(cx)? {
                Some(c) => c,
                None => break,
            };
        }
        if w.len() == 128 {
            return Err("read error".into());
        }
        Ok(w)
    }

    // R: serialize.c:InInteger
    #[inline]
    pub fn integer(&mut self, cx: &Cx) -> InResult<i32> {
        match self.fmt {
            Format::Ascii | Format::AsciiHex => {
                let w = self.in_word(cx)?;
                if w == b"NA" {
                    return Ok(NA_INTEGER);
                }
                scan_int(&w).ok_or_else(|| "read error".into())
            }
            Format::Binary => {
                let b = self.in_bytes(cx, 4)?;
                Ok(i32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
            }
            Format::Xdr => {
                let b = self.in_bytes(cx, 4)?;
                Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
            }
            Format::Any => Ok(NA_INTEGER),
        }
    }

    // R: serialize.c:InReal
    pub fn real(&mut self, cx: &Cx) -> InResult<f64> {
        match self.fmt {
            Format::Ascii | Format::AsciiHex => {
                let w = self.in_word(cx)?;
                match &w[..] {
                    b"NA" => Ok(robigo_heap::NA_REAL),
                    b"NaN" => Ok(f64::NAN),
                    b"Inf" => Ok(f64::INFINITY),
                    b"-Inf" => Ok(f64::NEG_INFINITY),
                    _ => scan_real(&w).ok_or_else(|| "read error".into()),
                }
            }
            Format::Binary => {
                let b = self.in_bytes(cx, 8)?;
                Ok(f64::from_ne_bytes(b.try_into().unwrap()))
            }
            Format::Xdr => {
                let b = self.in_bytes(cx, 8)?;
                Ok(f64::from_be_bytes(b.try_into().unwrap()))
            }
            Format::Any => Ok(robigo_heap::NA_REAL),
        }
    }

    // R: serialize.c:InString
    pub fn string(&mut self, cx: &Cx, length: usize) -> InResult<Vec<u8>> {
        if self.fmt == Format::Ascii || self.fmt == Format::AsciiHex {
            let mut buf = Vec::with_capacity(length);
            if length > 0 {
                // GetChar/UngetChar over one pending character
                let mut last: Option<u8> = None;
                fn get(s: &mut InStream, cx: &Cx, last: &mut Option<u8>) -> InResult<u8> {
                    if let Some(c) = last.take() {
                        return Ok(c);
                    }
                    // (EOF is GNU R's -1, which no escape matches)
                    Ok(s.in_char(cx)?.unwrap_or(0xff))
                }
                let mut c;
                loop {
                    c = get(self, cx, &mut last)?;
                    if !is_space(c) {
                        break;
                    }
                }
                last = Some(c);
                for _ in 0..length {
                    c = get(self, cx, &mut last)?;
                    if c == b'\\' {
                        c = get(self, cx, &mut last)?;
                        let b = match c {
                            b'n' => b'\n',
                            b't' => b'\t',
                            b'v' => 0x0b,
                            b'b' => 0x08,
                            b'r' => b'\r',
                            b'f' => 0x0c,
                            b'a' => 0x07,
                            b'\\' => b'\\',
                            b'?' => b'?',
                            b'\'' => b'\'',
                            b'"' => b'"',
                            b'0'..=b'7' => {
                                let mut d: u32 = 0;
                                let mut j = 0;
                                while (b'0'..b'8').contains(&c) && j < 3 {
                                    d = d * 8 + (c - b'0') as u32;
                                    c = get(self, cx, &mut last)?;
                                    j += 1;
                                }
                                last = Some(c);
                                d as u8
                            }
                            _ => c,
                        };
                        buf.push(b);
                    } else {
                        buf.push(c);
                    }
                }
            }
            Ok(buf)
        } else {
            Ok(self.in_bytes(cx, length)?.to_vec())
        }
    }

    /// A raw vector's bytes in the ascii format: one `%2x` word each.
    pub fn hex_byte(&mut self, cx: &Cx) -> InResult<u8> {
        let w = self.in_word(cx)?;
        let digits: Vec<u8> = w.iter().copied().take(2).take_while(|c| c.is_ascii_hexdigit()).collect();
        if digits.is_empty() {
            return Err("read error".into());
        }
        Ok(u8::from_str_radix(std::str::from_utf8(&digits).unwrap(), 16).unwrap())
    }

    pub fn ints_into(&mut self, cx: &Cx, out: &mut [i32]) -> InResult<()> {
        match self.fmt {
            Format::Binary => {
                let b = self.in_bytes(cx, out.len() * 4)?;
                for (o, c) in out.iter_mut().zip(b.chunks_exact(4)) {
                    *o = i32::from_ne_bytes([c[0], c[1], c[2], c[3]]);
                }
            }
            Format::Xdr => {
                let b = self.in_bytes(cx, out.len() * 4)?;
                for (o, c) in out.iter_mut().zip(b.chunks_exact(4)) {
                    *o = i32::from_be_bytes([c[0], c[1], c[2], c[3]]);
                }
            }
            _ => {
                for o in out.iter_mut() {
                    *o = self.integer(cx)?;
                }
            }
        }
        Ok(())
    }

    pub fn reals_into(&mut self, cx: &Cx, out: &mut [f64]) -> InResult<()> {
        match self.fmt {
            Format::Binary => {
                let b = self.in_bytes(cx, out.len() * 8)?;
                for (o, c) in out.iter_mut().zip(b.chunks_exact(8)) {
                    *o = f64::from_ne_bytes(c.try_into().unwrap());
                }
            }
            Format::Xdr => {
                let b = self.in_bytes(cx, out.len() * 8)?;
                for (o, c) in out.iter_mut().zip(b.chunks_exact(8)) {
                    *o = f64::from_be_bytes(c.try_into().unwrap());
                }
            }
            _ => {
                for o in out.iter_mut() {
                    *o = self.real(cx)?;
                }
            }
        }
        Ok(())
    }

    /// The bytes not read yet, for a memory stream.
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
}

/// `sscanf(word, "%d", &i)`: a decimal integer prefix.
fn scan_int(w: &[u8]) -> Option<i32> {
    let mut i = 0;
    let neg = match w.first() {
        Some(b'-') => {
            i = 1;
            true
        }
        Some(b'+') => {
            i = 1;
            false
        }
        _ => false,
    };
    let start = i;
    let mut v: i64 = 0;
    while i < w.len() && w[i].is_ascii_digit() {
        v = (v * 10 + (w[i] - b'0') as i64).min(1 << 40);
        i += 1;
    }
    if i == start {
        return None;
    }
    let v = if neg { -v } else { v };
    // (glibc's %d of an out-of-range value wraps as strtol's result cast)
    Some(v.clamp(i64::MIN, i64::MAX) as i32)
}

/// `sscanf(word, "%lg", &d)`: strtod's longest prefix (decimal, hex,
/// infinity, nan).
pub fn scan_real(w: &[u8]) -> Option<f64> {
    let s = std::str::from_utf8(w).ok()?;
    let (sign, body) = match s.as_bytes().first() {
        Some(b'-') => (-1.0, &s[1..]),
        Some(b'+') => (1.0, &s[1..]),
        _ => (1.0, s),
    };
    let lower = body.to_ascii_lowercase();
    if lower.starts_with("inf") {
        return Some(sign * f64::INFINITY);
    }
    if lower.starts_with("nan") {
        return Some(f64::NAN);
    }
    if let Some(h) = lower.strip_prefix("0x") {
        return parse_hex(h).map(|v| sign * v);
    }
    // the longest decimal prefix
    let b = body.as_bytes();
    let mut i = 0;
    let mut digits = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
        digits += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    let mut end = i;
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let k = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > k {
            end = j;
        }
    }
    body[..end].parse::<f64>().ok().map(|v| sign * v)
}

fn parse_hex(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut mant: u64 = 0;
    let mut exp: i64 = 0;
    let mut any = false;
    let mut dropped = false;
    while i < b.len() && b[i].is_ascii_hexdigit() {
        let d = (b[i] as char).to_digit(16).unwrap() as u64;
        if mant >> 56 == 0 {
            mant = mant << 4 | d;
        } else {
            exp += 4;
            dropped |= d != 0;
        }
        i += 1;
        any = true;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_hexdigit() {
            let d = (b[i] as char).to_digit(16).unwrap() as u64;
            if mant >> 56 == 0 {
                mant = mant << 4 | d;
                exp -= 4;
            } else {
                dropped |= d != 0;
            }
            i += 1;
            any = true;
        }
    }
    if !any {
        return None;
    }
    if i < b.len() && b[i] == b'p' {
        let mut j = i + 1;
        let neg = j < b.len() && b[j] == b'-';
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let mut e: i64 = 0;
        let k = j;
        while j < b.len() && b[j].is_ascii_digit() {
            e = (e * 10 + (b[j] - b'0') as i64).min(100_000);
            j += 1;
        }
        if j > k {
            exp += if neg { -e } else { e };
        }
    }
    let _ = dropped;
    // an exact value when the mantissa fits in 53 bits (what %a writes)
    Some(ldexp(mant as f64, exp))
}

/// `x * 2^e`, in steps that keep the partial products normal.
fn ldexp(mut x: f64, mut e: i64) -> f64 {
    while e > 1000 {
        x *= 2f64.powi(1000);
        e -= 1000;
    }
    while e < -1000 {
        x *= 2f64.powi(-1000);
        e += 1000;
    }
    x * 2f64.powi(e as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g16_matches_printf() {
        assert_eq!(fmt_g(1.0, 16), "1");
        assert_eq!(fmt_g(0.1, 16), "0.1");
        assert_eq!(fmt_g(1.0 / 3.0, 16), "0.3333333333333333");
        assert_eq!(fmt_g(1e-5, 16), "1e-05");
        assert_eq!(fmt_g(123456789012345678.0, 16), "1.234567890123457e+17");
        assert_eq!(fmt_g(-2.5, 16), "-2.5");
        assert_eq!(fmt_g(1e16, 16), "1e+16");
        assert_eq!(fmt_g(1e15, 16), "1000000000000000");
    }

    #[test]
    fn a_matches_glibc() {
        assert_eq!(fmt_a(1.0), "0x1p+0");
        assert_eq!(fmt_a(3.0), "0x1.8p+1");
        assert_eq!(fmt_a(0.1), "0x1.999999999999ap-4");
        assert_eq!(fmt_a(-0.0), "-0x0p+0");
        assert_eq!(fmt_a(f64::MIN_POSITIVE / 2.0), "0x0.8p-1022");
        for d in [1.0, 3.0, 0.1, 1e300, 5e-324, 123.456] {
            assert_eq!(scan_real(fmt_a(d).as_bytes()), Some(d));
        }
    }
}
