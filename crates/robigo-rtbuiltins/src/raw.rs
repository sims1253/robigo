//! raw.c's conversions between strings, raw vectors and code points:
//! `charToRaw`, `rawToChar`, `utf8ToInt`, `intToUtf8`.

use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Encoding, Out, RResult, Root, View};

const LENGTH1: &str = "argument must be a character vector of length 1";
const FIRST_ONLY: &str = "argument should be a character vector of length 1\nall but the first element will be ignored";

/// The first string of a character vector argument of length 1 (a warning
/// for more), as `charToRaw` and `utf8ToInt` take it.
fn first_string(cx: &mut Cx, x: Root<'_>) -> RResult<()> {
    let n = match x.bind(cx).view() {
        View::Str(v) => v.len(),
        _ => 0,
    };
    if n == 0 {
        return Err(cx.error(LENGTH1));
    }
    if n > 1 {
        cx.warning(FIRST_ONLY)?;
    }
    Ok(())
}

/// `.Internal(charToRaw(x))`: the bytes of `x`'s first string.
// R: raw.c:do_charToRaw
pub fn do_char_to_raw(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("charToRaw", true).check_arity(cx, call, args)?;
    first_string(cx, args.root(0))?;
    let bytes = args.get(cx, 0).as_vector::<Char<'_>>().unwrap().get(0).bytes().to_vec();
    cx.build(out, |cx| Ok(cx.alloc_copy(&bytes)?.finish().into()))
}

/// `.Internal(rawToChar(x, multiple))`: the bytes as one native string
/// (trailing nuls dropped; an embedded one is mkCharLenCE's error), or one
/// string per byte.
// R: raw.c:do_rawToChar
pub fn do_raw_to_char(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("rawToChar", true).check_arity(cx, call, args)?;
    let bytes: Vec<u8> = match args.get(cx, 0).view() {
        View::Raw(v) => (0..v.len()).map(|i| v.get(i)).collect(),
        _ => return Err(cx.error("argument 'x' must be a raw vector")),
    };
    let multiple = crate::util::as_logical(cx, args.get(cx, 1));
    if multiple == i32::MIN {
        return Err(cx.error("argument 'multiple' must be TRUE or FALSE"));
    }
    if multiple != 0 {
        // mkChar of a one-byte C string: a nul byte is ""
        return cx.build(out, |cx| {
            let mut v = cx.alloc_vector::<Char<'_>>(bytes.len())?;
            for (i, &b) in bytes.iter().enumerate() {
                let s: &[u8] = if b == 0 { &[] } else { std::slice::from_ref(&b) };
                v.set(i, cx.mk_char(s, Encoding::Native)?);
            }
            Ok(v.finish().into())
        });
    }
    // strip trailing nuls
    let end = bytes.iter().rposition(|&b| b != 0).map_or(0, |j| j + 1);
    let s = &bytes[..end];
    cx.build(out, |cx| {
        let c = cx.mk_char(s, Encoding::Native)?;
        let mut v = cx.alloc_vector::<Char<'_>>(1)?;
        v.set(0, c);
        Ok(v.finish().into())
    })
}

/// `mbrtoint(&w, s)`: the code point at the start of `s` and its length in
/// bytes; 0 at the end (a nul), -1 for an invalid sequence, -2 for one cut
/// short.
// R: raw.c:mbrtoint
fn mbrtoint(s: &[u8]) -> (i32, i32) {
    let at = |k: usize| s.get(k).copied().unwrap_or(0);
    let byte = at(0) as u32;
    let cont = |k: usize| at(k) & 0xC0 == 0x80;
    if byte == 0 {
        (0, 0)
    } else if byte < 0xC0 {
        (byte as i32, 1)
    } else if byte < 0xE0 {
        if at(1) == 0 {
            return (0, -2);
        }
        if cont(1) { ((((byte & 0x1F) << 6) | (at(1) as u32 & 0x3F)) as i32, 2) } else { (0, -1) }
    } else if byte < 0xF0 {
        if at(1) == 0 || at(2) == 0 {
            return (0, -2);
        }
        if cont(1) && cont(2) {
            let w = ((byte & 0x0F) << 12) | ((at(1) as u32 & 0x3F) << 6) | (at(2) as u32 & 0x3F);
            if (0xD800..=0xDFFF).contains(&w) { (0, -1) } else { (w as i32, 3) }
        } else {
            (0, -1)
        }
    } else if byte <= 0xF4 {
        if at(1) == 0 || at(2) == 0 || at(3) == 0 {
            return (0, -2);
        }
        if cont(1) && cont(2) && cont(3) {
            let w = ((byte & 0x07) << 18)
                | ((at(1) as u32 & 0x3F) << 12)
                | ((at(2) as u32 & 0x3F) << 6)
                | (at(3) as u32 & 0x3F);
            if w <= 0x10FFFF { (w as i32, 4) } else { (0, -1) }
        } else {
            (0, -1)
        }
    } else {
        (0, -1)
    }
}

/// `utf8Valid(s)`: whether `s` is valid UTF-8 (RFC 3629: no overlong
/// forms, surrogates or code points above U+10FFFF).
// R: util.c:utf8Valid, valid_utf8.h:valid_utf8
pub fn utf8_valid(s: &[u8]) -> bool {
    std::str::from_utf8(s).is_ok()
}

/// `.Internal(utf8ToInt(x))`: the code points of `x`'s first string; NA
/// for NA or a string that is not valid UTF-8.
// R: raw.c:do_utf8ToInt
pub fn do_utf8_to_int(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("utf8ToInt", true).check_arity(cx, call, args)?;
    first_string(cx, args.root(0))?;
    let (na, s) = {
        let c = args.get(cx, 0).as_vector::<Char<'_>>().unwrap().get(0);
        (c == cx.na_string(), c.bytes().to_vec())
    };
    if na || !utf8_valid(&s) {
        return cx.build_scalar(out, i32::MIN);
    }
    let mut ans = Vec::with_capacity(s.len());
    let mut rest = &s[..];
    let mut used = 0;
    while !rest.is_empty() {
        let (w, u) = mbrtoint(rest);
        used = u;
        if u <= 0 {
            break;
        }
        ans.push(w);
        rest = &rest[u as usize..];
    }
    if used < 0 {
        return Err(cx.error("invalid UTF-8 string"));
    }
    cx.build(out, |cx| Ok(cx.alloc_copy(&ans)?.finish().into()))
}

/// `inttomb(s, wc)`: the UTF-8 bytes of `wc` (none for 0), in the
/// original UTF-8's up-to-six-byte forms. Also util.c's `Rwcrtomb32`
/// (`wcrtomb` in a UTF-8 locale), the same algorithm.
// R: raw.c:inttomb, util.c:Rwcrtomb32
pub(crate) fn inttomb(wc: i32, out: &mut Vec<u8>) {
    const TABLE1: [u32; 6] = [0x7f, 0x7ff, 0xffff, 0x1fffff, 0x3ffffff, 0x7fffffff];
    const TABLE2: [u8; 6] = [0, 0xc0, 0xe0, 0xf0, 0xf8, 0xfc];
    let mut cvalue = wc as u32;
    if cvalue == 0 {
        return;
    }
    let i = TABLE1.iter().position(|&t| cvalue <= t).unwrap_or(TABLE1.len());
    let mut buf = [0u8; 7];
    for j in (1..=i).rev() {
        buf[j] = 0x80 | (cvalue & 0x3f) as u8;
        cvalue >>= 6;
    }
    buf[0] = TABLE2[i.min(5)] | cvalue as u8;
    out.extend_from_slice(&buf[..=i]);
}

/// `.Internal(intToUtf8(x, multiple, allow_surrogate_pairs))`: the code
/// points as one UTF-8 string (NA if any is not one; 0 omitted; with
/// `allow_surrogate_pairs`, a high and a low surrogate as one), or one
/// string per code point.
// R: raw.c:do_intToUtf8
pub fn do_int_to_utf8(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("intToUtf8", true).check_arity(cx, call, args)?;
    let x: Vec<i32> = cx.scope(|cx, s| {
        let x = s.capture(cx, |cx, o| crate::coerce::vector::coerce_vector(cx, args.root(0), INTSXP, o))?;
        if x.bind(cx).sexptype() != INTSXP {
            return Err(cx.error("argument 'x' must be an integer vector"));
        }
        let v = x.bind(cx).as_vector::<i32>().unwrap();
        Ok((0..v.len()).map(|i| v.get(i)).collect())
    })?;
    let multiple = crate::util::as_logical(cx, args.get(cx, 1));
    if multiple == i32::MIN {
        return Err(cx.error("argument 'multiple' must be TRUE or FALSE"));
    }
    let s_pair = crate::util::as_logical(cx, args.get(cx, 2));
    if s_pair == i32::MIN {
        return Err(cx.error("argument 'allow_surrogate_pairs' must be TRUE or FALSE"));
    }
    if multiple != 0 {
        if s_pair != 0 {
            cx.warning("allow_surrogate_pairs = TRUE is incompatible with multiple = TRUE and will be ignored")?;
        }
        let strs: Vec<Option<Vec<u8>>> = x
            .iter()
            .map(|&c| {
                (c != i32::MIN && c >= 0 && !(0xD800..=0xDFFF).contains(&c) && c <= 0x10FFFF).then(|| {
                    let mut b = Vec::new();
                    inttomb(c, &mut b);
                    b
                })
            })
            .collect();
        return cx.build(out, |cx| {
            let mut v = cx.alloc_vector::<Char<'_>>(strs.len())?;
            for (i, s) in strs.iter().enumerate() {
                v.set(
                    i,
                    match s {
                        Some(b) => cx.mk_char(b, Encoding::Utf8)?,
                        None => cx.na_string(),
                    },
                );
            }
            Ok(v.finish().into())
        });
    }
    // one string: NA if any code point is not one
    let mut na = false;
    let mut i = 0;
    while i < x.len() {
        let c = x[i];
        if c == i32::MIN || c < 0 || (0xDC00..=0xDFFF).contains(&c) || c > 0x10FFFF {
            na = true;
            break;
        } else if (0xD800..=0xDBFF).contains(&c) {
            if s_pair == 0 || i + 1 >= x.len() {
                na = true;
                break;
            }
            if (0xDC00..=0xDFFF).contains(&x[i + 1]) {
                i += 1;
            } else {
                na = true;
                break;
            }
        }
        i += 1;
    }
    if na {
        return cx.build(out, |cx| {
            let mut v = cx.alloc_vector::<Char<'_>>(1)?;
            v.set(0, cx.na_string());
            Ok(v.finish().into())
        });
    }
    let mut bytes = Vec::new();
    let mut i = 0;
    while i < x.len() {
        let mut c = x[i];
        if s_pair != 0 && (0xD800..=0xDBFF).contains(&c) {
            // all the validity checking has already been done
            i += 1;
            let (hi, lo) = ((c - 0xD800) as u32, (x[i] - 0xDC00) as u32);
            c = (0x10000 + (hi << 10) + lo) as i32;
        }
        inttomb(c, &mut bytes);
        i += 1;
    }
    cx.build(out, |cx| {
        let c = cx.mk_char(&bytes, Encoding::Utf8)?;
        let mut v = cx.alloc_vector::<Char<'_>>(1)?;
        v.set(0, c);
        Ok(v.finish().into())
    })
}
