//! Strings: the width of a string printed with escapes (`Rstrwid`,
//! `Rstrlen`), the common width of a vector (`formatString`), and the
//! escaped, quoted, padded text (`EncodeString`), from printutils.c and
//! format.c.

use std::borrow::Cow;

use crate::locale::{
    at, is_high_surrogate, isprint, iswprint, mbrtowc, ri18n_wcwidth, translate_char, utf8toucs, utf8toucs32,
};
use crate::{Enc, Locale, PrintParams, Str, cfmt};

/// `Rprt_adj`: how `EncodeString` pads to its width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Justify {
    Left,
    Right,
    Centre,
    /// No padding at all.
    None,
}

impl Justify {
    /// R's numbering (`justify` in `encodeString()`): 0 left, 1 right,
    /// 2 centre, 3 none.
    pub fn from_r(j: i32) -> Option<Justify> {
        Some(match j {
            0 => Justify::Left,
            1 => Justify::Right,
            2 => Justify::Centre,
            3 => Justify::None,
            _ => return None,
        })
    }
}

/// Decode the character at `i`: R's `utf8toucs` for strings read as UTF-8,
/// the locale's `mbrtowc` otherwise. Returns (`res`, `wc`, `k`): the
/// length (0 for the terminator, negative when invalid), the `wchar_t`,
/// and the code point (a surrogate pair combined).
#[inline]
fn decode(p: &[u8], i: usize, use_utf8: bool, loc: Locale) -> (i32, i32, u32) {
    let (res, wc) = if use_utf8 { utf8toucs(p, i) } else { mbrtowc(p, i, loc) };
    let k = if use_utf8 && is_high_surrogate(wc) { utf8toucs32(wc, p, i) } else { wc as u32 };
    (res, wc, k)
}

/// The ANSI escapes \a \b \f \n \r \t \v \0: their letter.
#[inline]
fn ansi_escape(c: u32) -> Option<u8> {
    Some(match c {
        0x07 => b'a',
        0x08 => b'b',
        0x0C => b'f',
        0x0A => b'n',
        0x0D => b'r',
        0x09 => b't',
        0x0B => b'v',
        0 => b'0',
        _ => return None,
    })
}

/// The width of `s` printed with escapes and with `quote` (0 for none, or
/// the quote byte) escaped, without the quotes. `ienc` is how the bytes
/// are read: `Utf8` as UTF-8 by R's decoder, `Bytes` byte by byte, others
/// in the locale's encoding. In a multibyte locale this counts display
/// width.
// R: printutils.c:Rstrwid
pub fn rstrwid(s: &[u8], ienc: Enc, quote: u8, loc: Locale) -> i32 {
    let slen = s.len();
    let mut len = 0i32;
    if ienc == Enc::Bytes {
        // not currently used for that encoding
        for &k in s {
            len += if (0x20..0x80).contains(&k) { 1 } else { 4 };
        }
        return len;
    }
    if loc.mbcs() || ienc == Enc::Utf8 {
        let use_utf8 = ienc == Enc::Utf8;
        let (mut p, mut i) = (0usize, 0usize);
        while i < slen {
            let (res, wc, k) = decode(s, p, use_utf8, loc);
            if res >= 0 {
                if (0x20..0x7F).contains(&k) && iswprint(k, loc) {
                    len += match wc as u8 {
                        b'\\' => 2,
                        b'\'' | b'"' | b'`' => {
                            if quote == at(s, p) {
                                2
                            } else {
                                1
                            }
                        }
                        _ => 1, // assumes these are all width 1
                    };
                    p += 1;
                } else if k < 0x80 {
                    // print in octal unless an ANSI escape
                    len += if ansi_escape(wc as u32).is_some() { 2 } else { 4 };
                    p += 1;
                } else {
                    len += if iswprint(k, loc) {
                        ri18n_wcwidth(k)
                    } else if k > 0xffff {
                        10
                    } else {
                        6
                    };
                    i += res as usize - 1;
                    p += res as usize;
                }
            } else {
                // invalid char
                len += 4;
                p += 1;
            }
            i += 1;
        }
    } else {
        // not MBCS nor marked as UTF-8
        for &c in s {
            len += if c < 0x80 {
                // ASCII
                if isprint(c, loc) {
                    match c {
                        b'\\' => 2,
                        b'\'' | b'"' | b'`' => {
                            if quote == c {
                                2
                            } else {
                                1
                            }
                        }
                        _ => 1,
                    }
                } else if ansi_escape(c as u32).is_some() {
                    2
                } else {
                    4 // print in octal
                }
            } else if isprint(c, loc) {
                // 8 bit char
                1
            } else {
                4
            };
        }
    }
    len
}

/// The width of `s` printed by [`encode_string`] with `quote`, without the
/// quotes (`NA_character_` is not a `Str`: its width is `na_width`).
// R: printutils.c:Rstrlen
pub fn rstrlen(s: Str, quote: u8, loc: Locale) -> i32 {
    match s.enc() {
        Enc::Utf8 | Enc::Bytes => rstrwid(s.bytes(), s.enc(), quote, loc),
        _ => {
            let p = translate_char(s, loc).expect("only bytes strings fail to translate");
            rstrwid(&p, Enc::Native, quote, loc)
        }
    }
}

/// The field width of the strings in `x` (`None` is `NA_character_`).
// R: format.c:formatString
pub fn format_string<'a>(x: impl IntoIterator<Item = Option<Str<'a>>>, quote: u8, pp: &PrintParams) -> i32 {
    let mut xmax = 0;
    for s in x {
        let l = match s {
            None => {
                if quote != 0 {
                    pp.na_width
                } else {
                    pp.na_width_noquote
                }
            }
            Some(s) => rstrlen(s, quote, pp.locale) + if quote != 0 { 2 } else { 0 },
        };
        xmax = xmax.max(l);
    }
    xmax
}

/// Append `s` (`None` is `NA_character_`) as R prints it: control and
/// unprintable characters escaped, `\` and `quote` (0 for none, or the
/// quote byte) escaped, in quotes, padded to `w` by `justify`.
///
/// The text is in the native encoding, except that with `keep_utf8` a
/// string marked UTF-8 stays UTF-8 (`encodeString()` passes `w - 1000000`
/// for those and marks the result UTF-8).
// R: printutils.c:EncodeString
pub fn encode_string(
    out: &mut Vec<u8>,
    s: Option<Str>,
    w: i32,
    quote: u8,
    justify: Justify,
    keep_utf8: bool,
    pp: &PrintParams,
) {
    let loc = pp.locale;
    let mut quote = quote;
    // (p, the bytes to encode; cnt, how many to read; i, the width they pad
    // by; ienc, how they are read)
    let (p, cnt, i, ienc): (Cow<[u8]>, usize, i32, Enc) = match s {
        None => {
            let na = if quote != 0 { pp.na_string } else { pp.na_string_noquote };
            // (Rstrlen(R_print.na_string, 0): the NA string's own mark,
            // read in the native encoding below, as getCharCE(NA_STRING))
            let i = rstrlen(na, 0, loc);
            quote = 0;
            (Cow::Borrowed(na.bytes()), na.bytes().len(), i, Enc::Native)
        }
        Some(s) if s.enc() == Enc::Bytes => {
            let raw = s.bytes();
            let mut cnt = raw.len();
            let mut pp = Vec::with_capacity(4 * cnt);
            for &k in raw {
                if (0x20..0x80).contains(&k) {
                    pp.push(k);
                    // (counts a '"' once more, whatever the quote, and then
                    // reads that many bytes: past the end)
                    if quote != 0 && k == b'"' {
                        cnt += 1;
                    }
                } else {
                    pp.extend_from_slice(format!("\\x{k:02x}").as_bytes());
                    cnt += 3;
                }
            }
            (Cow::Owned(pp), cnt, cnt as i32, Enc::Native)
        }
        Some(s) if keep_utf8 && s.enc() == Enc::Utf8 => {
            (Cow::Borrowed(s.bytes()), s.bytes().len(), rstrlen(s, quote, loc), Enc::Utf8)
        }
        Some(s) => {
            let p = translate_char(s, loc).expect("bytes strings are handled above");
            let (cnt, i) = match p {
                Cow::Borrowed(_) => (s.bytes().len(), rstrlen(s, quote, loc)),
                Cow::Owned(ref t) => (t.len(), rstrwid(t, Enc::Native, quote, loc)),
            };
            (p, cnt, i, Enc::Native)
        }
    };
    let p = &*p;

    let mut b = w - i - if quote != 0 { 2 } else { 0 }; // total amount of padding
    if justify == Justify::None {
        b = 0;
    }
    if b > 0 && justify != Justify::Left {
        let b0 = if justify == Justify::Centre { b / 2 } else { b };
        cfmt::spaces(out, b0 as usize);
        b -= b0;
    }
    if quote != 0 {
        out.push(quote);
    }
    if fast_ascii(p, cnt, quote) {
        out.extend_from_slice(p);
    } else if loc.mbcs() || ienc == Enc::Utf8 {
        encode_mbcs(out, p, cnt, quote, ienc == Enc::Utf8, loc);
    } else {
        encode_single_byte(out, p, cnt, quote, loc);
    }
    if quote != 0 {
        out.push(quote);
    }
    if b > 0 && justify != Justify::Right {
        cfmt::spaces(out, b as usize);
    }
}

/// Printable ASCII with nothing to escape: copied as is by either loop.
#[inline]
fn fast_ascii(p: &[u8], cnt: usize, quote: u8) -> bool {
    cnt == p.len() && p.iter().all(|&c| (0x20..0x7F).contains(&c) && c != b'\\' && c != quote)
}

/// EncodeString's loop in a multibyte locale or for UTF-8 read by R's
/// decoder.
// R: printutils.c:EncodeString
fn encode_mbcs(out: &mut Vec<u8>, p: &[u8], cnt: usize, quote: u8, use_utf8: bool, loc: Locale) {
    let (mut pi, mut i) = (0usize, 0usize);
    while i < cnt {
        let (res, mut wc, mut k) = decode(p, pi, use_utf8, loc);
        // res = 0 is a terminator; some mbrtowc implementations return wc
        // past end of UCS
        if res >= 0 && (0..=0x10FFFF).contains(&wc) {
            // To be portable, treat \0 explicitly
            if res == 0 {
                k = 0;
                wc = 0;
            }
            if (0x20..0x7F).contains(&k) && iswprint(k, loc) {
                match wc as u8 {
                    b'\\' => {
                        out.extend_from_slice(b"\\\\");
                        pi += 1;
                    }
                    b'\'' | b'"' | b'`' => {
                        if quote == at(p, pi) {
                            out.push(b'\\');
                        }
                        out.push(at(p, pi));
                        pi += 1;
                    }
                    _ => {
                        for _ in 0..res {
                            out.push(at(p, pi));
                            pi += 1;
                        }
                    }
                }
            } else if k < 0x80 {
                // ANSI Escapes
                match ansi_escape(wc as u32) {
                    Some(e) => out.extend_from_slice(&[b'\\', e]),
                    // print in octal
                    None => out.extend_from_slice(format!("\\{:03o}", k as u8).as_bytes()),
                }
                pi += 1;
            } else {
                // wc could be an unpaired surrogate and this does not do
                // the same as Rstrwid
                if iswprint(k, loc) {
                    for _ in 0..res {
                        out.push(at(p, pi));
                        pi += 1;
                    }
                } else {
                    if k > 0xffff {
                        out.extend_from_slice(format!("\\U{{{k:06x}}}").as_bytes());
                    } else {
                        out.extend_from_slice(format!("\\u{k:04x}").as_bytes());
                    }
                    pi += res as usize;
                }
                i += res as usize - 1;
            }
        } else {
            // invalid char
            out.extend_from_slice(format!("\\x{:02x}", at(p, pi)).as_bytes());
            pi += 1;
        }
        i += 1;
    }
}

/// EncodeString's loop in a single-byte locale.
// R: printutils.c:EncodeString
fn encode_single_byte(out: &mut Vec<u8>, p: &[u8], cnt: usize, quote: u8, loc: Locale) {
    for i in 0..cnt {
        let c = at(p, i);
        if c < 0x80 {
            // ASCII
            if c != b'\t' && isprint(c, loc) {
                // Windows has \t as printable
                match c {
                    b'\\' => out.extend_from_slice(b"\\\\"),
                    b'\'' | b'"' | b'`' => {
                        if quote == c {
                            out.push(b'\\');
                        }
                        out.push(c);
                    }
                    _ => out.push(c),
                }
            } else {
                // ANSI Escapes, or print in octal
                match ansi_escape(c as u32) {
                    Some(e) => out.extend_from_slice(&[b'\\', e]),
                    None => out.extend_from_slice(format!("\\{c:03o}").as_bytes()),
                }
            }
        } else if !isprint(c, loc) {
            // 8 bit char: print in octal
            out.extend_from_slice(format!("\\{c:03o}").as_bytes());
        } else {
            out.push(c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(b: &[u8], e: Enc, quote: u8, loc: Locale) -> String {
        let pp = PrintParams { locale: loc, ..PrintParams::DEFAULT };
        let mut v = Vec::new();
        encode_string(&mut v, Some(Str::new(b, e)), 0, quote, Justify::Left, false, &pp);
        String::from_utf8_lossy(&v).into_owned()
    }

    // Values from GNU R 4.6.2 (print()); tests/oracle.rs has the rest.
    #[test]
    fn basics() {
        assert_eq!(enc(b"a\"b\n", Enc::Native, b'"', Locale::Utf8), "\"a\\\"b\\n\"");
        assert_eq!(enc("caf\u{e9}".as_bytes(), Enc::Utf8, b'"', Locale::C), "\"caf<U+00E9>\"");
        assert_eq!(enc(b"\xe9", Enc::Latin1, 0, Locale::Utf8), "\u{e9}");
        assert_eq!(enc(b"\xff", Enc::Native, 0, Locale::Utf8), "\\xff");
        assert_eq!(enc(b"\xff", Enc::Native, 0, Locale::C), "\\377");
        assert_eq!(enc("\u{2028}".as_bytes(), Enc::Utf8, 0, Locale::Utf8), "\\u2028");
    }

    #[test]
    fn justify() {
        let pp = PrintParams::DEFAULT;
        let s = Some(Str::new(b"ab", Enc::Native));
        let run = |j| {
            let mut v = Vec::new();
            encode_string(&mut v, s, 7, b'"', j, false, &pp);
            String::from_utf8(v).unwrap()
        };
        assert_eq!(run(Justify::Left), "\"ab\"   ");
        assert_eq!(run(Justify::Right), "   \"ab\"");
        assert_eq!(run(Justify::Centre), " \"ab\"  ");
        assert_eq!(run(Justify::None), "\"ab\"");
    }
}
