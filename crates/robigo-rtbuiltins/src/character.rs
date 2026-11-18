//! character.c's `.Internal(nchar(x, type, allowNA, keepNA))`: `do_nchar`
//! and `R_nchar`.

use crate::arith::names_dim_dimnames;
use crate::attrib::{self, Attrs};
use crate::coerce::{Strings, Uncoercible};
use crate::text::to_native;
use crate::util::{as_logical, not_yet};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Built, Char, Cx, Diag, Dispatched, Encoding, Generic, Lgl, Out, RResult, Root};

const NA_INTEGER: i32 = i32::MIN;
use robigo_text::{Enc, Str};

/// What `nchar` counts (Defn.h `nchar_type`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NcharType {
    Bytes,
    Chars,
    Width,
}

/// `R_nchar(string, type, allowNA, keepNA)`'s outcome for one string.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Nchar {
    Count(i32),
    /// GNU R's -1: invalid multibyte string
    Invalid,
    /// GNU R's -2: not computable in "bytes" encoding
    Bytes,
}

/// `R_nchar(string, type_, allowNA, keepNA, NULL)` for a string (None:
/// `NA_STRING`) in the locale `loc`.
// R: character.c:R_nchar
pub fn r_nchar(s: Option<Str<'_>>, ty: NcharType, allow_na: bool, keep_na: bool, loc: robigo_rtcore::Locale) -> Nchar {
    const NA: i32 = i32::MIN;
    let Some(s) = s else {
        return Nchar::Count(if keep_na { NA } else { 2 });
    };
    let p = s.bytes();
    let invalid = |allow: bool| if allow { Nchar::Count(NA) } else { Nchar::Invalid };
    let bytes = |allow: bool| if allow { Nchar::Count(NA) } else { Nchar::Bytes };
    // ASCII strings carry no mark: they are native
    match ty {
        NcharType::Bytes => Nchar::Count(p.len() as i32),
        NcharType::Chars => match s.enc() {
            Enc::Utf8 => match std::str::from_utf8(p) {
                Ok(t) => Nchar::Count(t.chars().count() as i32),
                Err(_) => invalid(allow_na),
            },
            Enc::Latin1 => Nchar::Count(p.len() as i32),
            Enc::Bytes => bytes(allow_na),
            Enc::Native if loc.utf8 => match mbstowcs_len(&to_native(s, loc).expect("robigo: native")) {
                Some(n) => Nchar::Count(n as i32),
                None => invalid(allow_na),
            },
            Enc::Native => Nchar::Count(to_native(s, loc).expect("robigo: native").len() as i32),
        },
        NcharType::Width => match s.enc() {
            Enc::Utf8 => match std::str::from_utf8(p) {
                Ok(_) => {
                    let mut nc = 0;
                    let mut i = 0;
                    while i < p.len() {
                        let (len, wc) = robigo_text::utf8toucs(p, i);
                        let ucs = if robigo_text::is_high_surrogate(wc) {
                            robigo_text::utf8toucs32(wc, p, i)
                        } else {
                            wc as u32
                        };
                        let _ = len;
                        nc += robigo_text::ri18n_wcwidth(ucs);
                        i += utf8clen(p[i]);
                    }
                    Nchar::Count(nc)
                }
                Err(_) => invalid(allow_na),
            },
            Enc::Bytes => bytes(allow_na),
            Enc::Latin1 => Nchar::Count(p.len() as i32),
            Enc::Native if loc.utf8 => {
                let t = to_native(s, loc).expect("robigo: native");
                match mbstowcs(&t) {
                    Some(wcs) => {
                        // Ri18n_wcswidth, which is -1 for a character of
                        // width -1 (then the count of characters)
                        let mut w = 0i32;
                        let mut bad = false;
                        for &c in &wcs {
                            let now = robigo_text::ri18n_wcwidth(c);
                            if now == -1 {
                                bad = true;
                                break;
                            }
                            w += now;
                        }
                        Nchar::Count(if bad { wcs.len() as i32 } else { w })
                    }
                    None => invalid(allow_na),
                }
            }
            Enc::Native => Nchar::Count(to_native(s, loc).expect("robigo: native").len() as i32),
        },
    }
}

/// `utf8clen(c)`: the length of the UTF-8 character starting with `c`.
// R: util.c:utf8clen
fn utf8clen(c: u8) -> usize {
    if c & 0xc0 != 0xc0 {
        return 1;
    }
    1 + match c & 0x3f {
        0x00..=0x1f => 1,
        0x20..=0x2f => 2,
        0x30..=0x37 => 3,
        0x38..=0x3b => 4,
        _ => 5,
    }
}

/// glibc's `mbstowcs` in a UTF-8 locale: the characters, or None for an
/// invalid string.
fn mbstowcs(s: &[u8]) -> Option<Vec<u32>> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let (len, wc) = robigo_text::mbrtowc_utf8(s, i);
        if len <= 0 {
            return None;
        }
        out.push(wc as u32);
        i += len as usize;
    }
    Some(out)
}

/// `mbstowcs(NULL, s, 0)`: the number of characters, or None.
fn mbstowcs_len(s: &[u8]) -> Option<usize> {
    mbstowcs(s).map(|v| v.len())
}

/// `do_nchar`.
// R: character.c:do_nchar
pub fn do_nchar(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let nargs = args.len();
    if !(3..=4).contains(&nargs) {
        let s = if nargs == 1 { "" } else { "s" };
        return Err(cx.error(&format!("{nargs} argument{s} passed to 'nchar' which requires 3 to 4")));
    }
    let out = match cx.try_dispatch(&NCHAR, call, args, env, out)? {
        Dispatched::Done => return Ok(()),
        Dispatched::Default(out) => out,
    };
    let x = args.root(0);
    let requires = "'nchar()' requires a character vector";
    if x.bind(cx).sexptype() == INTSXP && attrib::inherits(x.bind(cx), b"factor") {
        return Err(cx.error(requires));
    }
    match Strings::of(cx, x.bind(cx)).map(|_| ()) {
        Ok(()) => {}
        Err(e @ (Uncoercible::Type(_) | Uncoercible::Env | Uncoercible::Deparse(_))) => {
            return Err(cx.error(&e.message()));
        }
        Err(Uncoercible::NotYet(what)) => return Err(not_yet(cx, Some(call), what)),
    }
    let ty = {
        let st = args.get(cx, 1);
        let t = st.as_vector::<Char<'_>>().and_then(|v| v.as_slice()).filter(|s| s.len() == 1).map(|s| s[0].bytes());
        match t {
            Some(t) if !t.is_empty() && b"bytes".starts_with(t) => Some(NcharType::Bytes),
            Some(t) if !t.is_empty() && b"chars".starts_with(t) => Some(NcharType::Chars),
            Some(t) if !t.is_empty() && b"width".starts_with(t) => Some(NcharType::Width),
            _ => None,
        }
    };
    let Some(ty) = ty else {
        return Err(cx.error("invalid 'type' argument"));
    };
    let allow_na = match as_logical(cx, args.get(cx, 2)) {
        i32::MIN => false,
        v => v != 0,
    };
    let keep_na = if nargs >= 4 {
        match as_logical(cx, args.get(cx, 3)) {
            i32::MIN => ty != NcharType::Width,
            v => v != 0,
        }
    } else {
        ty != NcharType::Width
    };
    let loc = cx.locale();
    // allocVector(INTSXP, len), then the counts into it: an element GNU R
    // cannot count ends the attempt with its error, raised after it
    cx.build_with(out, |cx| {
        let v = Strings::of(cx, x.bind(cx)).expect("robigo: checked");
        let mut r = cx.alloc_vector::<i32>(v.len())?;
        for (i, slot) in r.as_mut_slice().iter_mut().enumerate() {
            match r_nchar(v.get(i), ty, allow_na, keep_na, loc) {
                Nchar::Count(n) => *slot = n,
                e => return Ok(Built::fail(Diag::error(nchar_error(e, ty, i)))),
            }
        }
        let mut a = Attrs::new();
        // the names, dim and dimnames of coerceVector(x, STRSXP), which
        // keeps x's attributes
        names_dim_dimnames(cx, &mut a, x.bind(cx))?;
        a.apply(cx, &mut r)?;
        Ok(Built::done(r.finish()))
    })
}

/// do_nchar's error for element `i` (0-based) that `R_nchar` could not
/// count.
// R: character.c:do_nchar
fn nchar_error(e: Nchar, ty: NcharType, i: usize) -> String {
    match (e, ty) {
        (Nchar::Invalid, _) => format!("invalid multibyte string, element {}", i + 1),
        (_, NcharType::Chars) => {
            format!("number of characters is not computable in \"bytes\" encoding, element {}", i + 1)
        }
        _ => format!("width is not computable in \"bytes\" encoding, element {}", i + 1),
    }
}

/// `DispatchOrEval(call, op, "nchar", args, env, &ans, 0, 1)`.
const NCHAR: Generic = Generic::builtin("nchar");

// ---- strrep ---------------------------------------------------------------------

/// `.Internal(strrep(x, times))`: each string repeated, `x` and `times`
/// recycled to the longer, NA for an NA of either; the names of `x` kept
/// when it is not recycled. base's `strrep` passes a character `x` and an
/// integer `times`.
// R: character.c:do_strrep
pub fn do_strrep(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("strrep", true).check_arity(cx, call, args)?;
    let (x, n) = (args.root(0), args.root(1));
    // (a package ALTREP vector has no length or data robigo reads)
    if x.bind(cx).is_package_altrep() || n.bind(cx).is_package_altrep() {
        return Err(crate::util::not_yet(cx, Some(call), "strrep() of a package ALTREP vector"));
    }
    let length = |cx: &Cx, v: Root<'_>| {
        let v = v.bind(cx);
        v.len().ok_or_else(|| format!("LENGTH or similar applied to {} object", crate::util::type2char(v.sexptype())))
    };
    let (nx, nn) = match (length(cx, x), length(cx, n)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(m), _) | (_, Err(m)) => return Err(cx.error(&m)),
    };
    if nx == 0 || nn == 0 {
        return cx.build(out, |cx| Ok(cx.alloc_vector::<Char<'_>>(0)?.finish().into()));
    }
    // STRING_ELT(x, .) and INTEGER(n) of anything else
    for (v, accessor, what, ty) in [(x, "STRING_ELT", "character vector", STRSXP), (n, "INTEGER", "integer", INTSXP)] {
        let t = v.bind(cx).sexptype();
        if t != ty {
            let m = format!("{accessor}() can only be applied to a '{what}', not a '{}'", crate::util::type2char(t));
            return Err(cx.error(&m));
        }
    }
    let ns = nx.max(nn);
    cx.build_with(out, |cx| {
        let (xv, nv) = (x.bind(cx), n.bind(cx));
        let xs = xv.as_vector::<Char<'_>>().unwrap();
        let times = nv.as_ints().unwrap();
        let mut s = cx.alloc_vector::<Char<'_>>(ns)?;
        for i in 0..ns {
            let (el, ni) = (xs.get(i % nx), times[i % nn]);
            if el == cx.na_string() || ni == NA_INTEGER {
                s.set(i, cx.na_string());
                continue;
            }
            if ni < 0 {
                return Ok(Built::fail(Diag::error("invalid 'times' value")));
            }
            let b = el.bytes();
            // a feasible length, in double against integer overflow
            if b.len() as f64 * ni as f64 > i32::MAX as f64 {
                return Ok(Built::fail(Diag::error("R character strings are limited to 2^31-1 bytes")));
            }
            s.set(i, cx.mk_char(&b.repeat(ni as usize), el.enc())?);
        }
        // the names, if not recycled
        if ns == nx
            && let Some(d) = attrib::names(xv)
        {
            let mut a = Attrs::new();
            a.install(attrib::Attr { tag: cx.sym(robigo_rtcore::RSym::Names), value: d });
            a.apply(cx, &mut s)?;
        }
        Ok(Built::done(s.finish()))
    })
}

// ---- substr, startsWith (B2b) ------------------------------------------------

/// `.Internal(substr(x, start, stop))`: characters `start..=stop` of each
/// string (counted in UTF-8 characters for UTF-8 and native strings in a
/// UTF-8 locale, else bytes), the attributes kept.
// R: character.c:do_substr, substr
pub fn do_substr(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("substr", true).check_arity(cx, call, args)?;
    let x = args.root(0);
    if x.bind(cx).sexptype() != STRSXP {
        return Err(cx.error("extracting substrings from a non-character object"));
    }
    if x.bind(cx).is_package_altrep() {
        return Err(crate::util::not_yet(cx, Some(call), "substr() of a package ALTREP vector"));
    }
    let len = x.bind(cx).len().unwrap_or(0);
    if len > 0 {
        let (sa, so) = (args.get(cx, 1), args.get(cx, 2));
        let k = sa.len().unwrap_or(0);
        let l = so.len().unwrap_or(0);
        if sa.sexptype() != INTSXP || k == 0 || (so.sexptype() != NILSXP && (so.sexptype() != INTSXP || l == 0)) {
            return Err(cx.error("invalid substring arguments"));
        }
    }
    let mbcs = cx.locale().utf8;
    cx.build_with(out, |cx| {
        let xv = x.bind(cx);
        let v = xv.as_vector::<Char<'_>>().unwrap();
        let mut s = cx.alloc_vector::<Char<'_>>(len)?;
        if len > 0 {
            let starts = args.get(cx, 1).as_ints().unwrap();
            let so = args.get(cx, 2);
            let max = [i32::MAX];
            let stops: &[i32] = if so.sexptype() == NILSXP { &max } else { so.as_ints().unwrap() };
            for i in 0..len {
                let (mut start, stop) = (starts[i % starts.len()], stops[i % stops.len()]);
                let el = v.get(i);
                if el == cx.na_string() || start == NA_INTEGER || stop == NA_INTEGER {
                    s.set(i, cx.na_string());
                    continue;
                }
                if start < 1 {
                    start = 1;
                }
                if start > stop {
                    s.set(i, cx.blank_string());
                    continue;
                }
                let b = el.bytes();
                let enc = el.enc();
                let by_char = enc == Encoding::Utf8 || (!el.is_ascii() && enc == Encoding::Native && mbcs);
                let (from, to) = if by_char {
                    if std::str::from_utf8(b).is_err() {
                        return Ok(Built::fail(Diag::error(format!("invalid multibyte string, element {}", i + 1))));
                    }
                    let mut p = 0;
                    let mut ci = 0;
                    while ci < start - 1 && p < b.len() {
                        p += utf8clen(b[p]);
                        ci += 1;
                    }
                    let from = p.min(b.len());
                    while ci < stop && p < b.len() {
                        p += utf8clen(b[p]);
                        ci += 1;
                    }
                    (from, p.min(b.len()))
                } else {
                    let (sa, so) = (start as usize, stop as usize);
                    if so - 1 < b.len() {
                        (sa - 1, so)
                    } else if sa - 1 < b.len() {
                        (sa - 1, b.len())
                    } else {
                        (0, 0)
                    }
                };
                s.set(i, cx.mk_char(&b[from..to], enc)?);
            }
        }
        let mut a = crate::attrib::Attrs::new();
        a.copy_all(xv);
        a.apply(cx, &mut s)?;
        Ok(Built::done(s.finish()))
    })
}

/// `.Internal(startsWith(x, prefix))` (`CODE` 0), `endsWith` (1).
// R: character.c:do_startsWith
pub fn do_starts_with<const CODE: i32>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let row = if CODE == 0 { crate::row!("startsWith", true) } else { crate::row!("endsWith", true) };
    row.check_arity(cx, call, args)?;
    let (x, fix) = (args.root(0), args.root(1));
    if x.bind(cx).sexptype() != STRSXP || fix.bind(cx).sexptype() != STRSXP {
        return Err(cx.error("non-character object(s)"));
    }
    let loc = cx.locale();
    cx.build(out, |cx| {
        let xv = x.bind(cx).as_vector::<Char<'_>>().unwrap();
        let fv = fix.bind(cx).as_vector::<Char<'_>>().unwrap();
        let (n1, n2) = (xv.len(), fv.len());
        let n = if n1 > 0 && n2 > 0 { n1.max(n2) } else { 0 };
        let na = cx.na_string();
        let tr = |c: Char<'_>| -> Option<Vec<u8>> {
            (c != na).then(|| {
                crate::text::to_utf8(crate::coerce::text(c), loc)
                    .map(|b| b.into_owned())
                    .unwrap_or_else(|_| c.bytes().to_vec())
            })
        };
        let xs: Vec<Option<Vec<u8>>> = (0..n1).map(|i| tr(xv.get(i))).collect();
        let ys: Vec<Option<Vec<u8>>> = (0..n2).map(|i| tr(fv.get(i))).collect();
        let r = cx.alloc_with(n, |i| {
            Lgl(match (&xs[i % n1.max(1)], &ys[i % n2.max(1)]) {
                (Some(a), Some(b)) => (if CODE == 0 { a.starts_with(b) } else { a.ends_with(b) }) as i32,
                _ => NA_INTEGER,
            })
        })?;
        Ok(r.finish().into())
    })
}

/// `nzchar(x, keepNA = FALSE)`: whether each string of `x` (coerced to
/// character) is not empty; NA is TRUE (its "NA"), or NA with `keepNA`.
// R: character.c:do_nzchar
pub fn do_nzchar(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let nargs = args.len();
    if !(1..=2).contains(&nargs) {
        let msg = if nargs == 1 {
            format!("{nargs} argument passed to 'nzchar' which requires 1 to 2")
        } else {
            format!("{nargs} arguments passed to 'nzchar' which requires 1 to 2")
        };
        return Err(cx.error_call(Some(call), &msg));
    }
    crate::util::check1arg(cx, args, call, "x")?;
    let x0 = args.get(cx, 0);
    if x0.sexptype() == INTSXP && attrib::inherits(x0, b"factor") {
        return Err(cx.error("'nzchar()' requires a character vector"));
    }
    let keep_na = nargs > 1 && {
        let k = as_logical(cx, args.get(cx, 1));
        k != i32::MIN && k != 0
    };
    let x = args.root(0);
    cx.build_with(out, |cx| {
        let s = match crate::coerce::vector::coerce(cx, x.bind(cx), STRSXP, false)? {
            Built::Done(v, _) => v,
            Built::Fail(d) => return Ok(Built::fail(d)),
        };
        let Some(v) = s.as_vector::<Char<'_>>() else {
            return Ok(Built::fail(Diag::error("'nzchar()' requires a character vector")));
        };
        let na = cx.na_string();
        let ans: Vec<Lgl> = (0..v.len())
            .map(|i| {
                let c = v.get(i);
                if keep_na && c == na { Lgl::NA } else { Lgl(!c.bytes().is_empty() as i32) }
            })
            .collect();
        Ok(Built::done(cx.alloc_copy(&ans)?.finish()))
    })
}

/// `.Internal(encodeString(x, width, quote, justify, na.encode))`: each
/// string as `print` shows it (escaped, quoted, padded to `width`, or to
/// the widest when NA), a UTF-8 one kept UTF-8; NA kept as NA unless
/// `na.encode`.
// R: util.c:do_encodeString
pub fn do_encode_string(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    use robigo_text::string::{Justify, encode_string, rstrlen};
    crate::row!("encodeString", true).check_arity(cx, call, args)?;
    let x = args.get(cx, 0);
    if x.sexptype() != STRSXP {
        return Err(cx.error("a character vector argument expected"));
    }
    const NA: i32 = i32::MIN;
    let wa = args.get(cx, 1);
    let mut w = if wa.sexptype() == NILSXP { NA } else { crate::coerce::vector::as_integer(cx, wa).0 };
    if w != NA && w < 0 {
        return Err(cx.error("invalid 'width' value"));
    }
    let q = args.get(cx, 2);
    let qv = q.as_vector::<Char<'_>>().filter(|v| v.len() == 1);
    let Some(qv) = qv else {
        return Err(cx.error("invalid 'quote' value"));
    };
    let loc = cx.locale();
    let cs = match to_native(crate::coerce::text(qv.get(0)), loc) {
        Ok(c) => c.into_owned(),
        Err(_) => return Err(cx.error("translating strings with \"bytes\" encoding is not allowed")),
    };
    let quote = cs.first().copied().unwrap_or(0);
    if cs.len() > 1 {
        cx.warning("only the first character of 'quote' will be used")?;
    }
    let (justify, _) = crate::coerce::vector::as_integer(cx, args.get(cx, 3));
    if justify == NA || !(0..=3).contains(&justify) {
        return Err(cx.error("invalid 'justify' value"));
    }
    if justify == 3 {
        w = 0;
    }
    let na = as_logical(cx, args.get(cx, 4));
    if na == NA {
        return Err(cx.error("invalid 'na.encode' value"));
    }
    let na = na != 0;
    let pp = robigo_text::PrintParams { locale: crate::text::ctype(loc), ..robigo_text::PrintParams::DEFAULT };
    let jt = [Justify::Left, Justify::Right, Justify::Centre, Justify::None][justify as usize];
    let x = args.root(0);
    cx.build(out, |cx| {
        let xv = x.bind(cx).as_vector::<Char<'_>>().unwrap();
        let nas = cx.na_string();
        let mut w = w;
        if w == NA && justify < 3 {
            w = 0;
            for i in 0..xv.len() {
                let c = xv.get(i);
                if na || c != nas {
                    let t = if c == nas { robigo_text::Str::ascii(b"NA") } else { crate::coerce::text(c) };
                    w = w.max(rstrlen(t, quote, pp.locale));
                }
            }
            if quote != 0 {
                w += 2;
            }
        }
        let mut ans = cx.alloc_vector::<Char<'_>>(xv.len())?;
        let mut b = Vec::new();
        for i in 0..xv.len() {
            let c = xv.get(i);
            if !na && c == nas {
                ans.set(i, c);
                continue;
            }
            b.clear();
            let utf8 = c != nas && c.enc() == Encoding::Utf8;
            let s = (c != nas).then(|| crate::coerce::text(c));
            encode_string(&mut b, s, w, quote, jt, utf8, &pp);
            ans.set(i, cx.mk_char(&b, if utf8 { Encoding::Utf8 } else { Encoding::Native })?);
        }
        Ok(ans.finish().into())
    })
}

/// `translateChar(s)` of a string or `NA_STRING` ("NA"), or GNU R's error
/// for a "bytes" one.
fn native_bytes(cx: &mut Cx, x: Root<'_>, i: usize) -> RResult<Vec<u8>> {
    let r = {
        let c = x.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
        to_native(crate::coerce::text(c), cx.locale()).map(|b| b.into_owned())
    };
    r.map_err(|_| cx.error("translating strings with \"bytes\" encoding is not allowed"))
}

/// `iswctype(wc, class)` in the C library's locale (`isalpha` and the like
/// of a byte in a single-byte locale: ASCII only, the C locale's).
#[inline]
fn isw(class: robigo_text::tre::Class, wc: u32, loc: robigo_text::Locale) -> bool {
    class.contains(wc, loc)
}

/// glibc's `mbrtowc(&wc, s + i, n, ps)` in a UTF-8 locale: the length of
/// the character and the character; 0 at the end, negative for an invalid
/// sequence or one longer than `n` bytes.
fn mbrtowc_n(s: &[u8], i: usize, n: usize) -> (isize, i32) {
    if i >= s.len() {
        return (0, 0);
    }
    if n == 0 {
        return (-2, 0);
    }
    let (used, wc) = robigo_text::mbrtowc_utf8(s, i);
    if used > 0 && used as usize > n { (-2, 0) } else { (used as isize, wc) }
}

/// `Mbrtowc(&wc, s + i, n, &mb_st)`: `mbrtowc`, or GNU R's error with the
/// rest of the string written out, each byte that does not start a
/// character (within the bytes left of `n`) as `<xx>`.
// R: util.c:Mbrtowc
fn r_mbrtowc(cx: &mut Cx, s: &[u8], i: usize, n: usize) -> RResult<(usize, i32)> {
    let (used, wc) = mbrtowc_n(s, i, n);
    if used >= 0 {
        return Ok((used as usize, wc));
    }
    let (mut p, mut n, mut used) = (i, n, used);
    let mut err = Vec::new();
    while p < s.len() {
        // don't do the first to keep ps state straight
        if p > i {
            used = mbrtowc_n(s, p, n).0;
        }
        if used == 0 {
            break;
        } else if used > 0 {
            err.extend_from_slice(&s[p..p + used as usize]);
            p += used as usize;
            n -= used as usize;
        } else {
            err.extend_from_slice(format!("<{:02x}>", s[p]).as_bytes());
            p += 1;
            n = n.wrapping_sub(1);
        }
    }
    Err(cx.error(&format!("invalid multibyte string at '{}'", String::from_utf8_lossy(&err))))
}

/// `wcrtomb(s, wc, ps)` in a UTF-8 locale (and R's `Rwcrtomb32`): `wc` in
/// UTF-8, in the original up-to-six-byte forms; nothing for 0.
// R: util.c:Rwcrtomb32
fn wcrtomb_utf8(wc: u32, out: &mut Vec<u8>) {
    const TABLE1: [u32; 6] = [0x7f, 0x7ff, 0xffff, 0x1fffff, 0x3ffffff, 0x7fffffff];
    const TABLE2: [u8; 6] = [0, 0xc0, 0xe0, 0xf0, 0xf8, 0xfc];
    let mut cvalue = wc;
    if cvalue == 0 {
        return;
    }
    let i = TABLE1.iter().position(|&t| cvalue <= t).unwrap_or(TABLE1.len()).min(5);
    let mut buf = [0u8; 6];
    for j in (1..=i).rev() {
        buf[j] = 0x80 | (cvalue & 0x3f) as u8;
        cvalue >>= 6;
    }
    buf[0] = TABLE2[i] | cvalue as u8;
    out.extend_from_slice(&buf[..=i]);
}

/// gram.y's reserved words (`keywords[]`, but `...`).
const RESERVED: [&[u8]; 19] = [
    b"NULL",
    b"NA",
    b"TRUE",
    b"FALSE",
    b"Inf",
    b"NaN",
    b"NA_integer_",
    b"NA_real_",
    b"NA_character_",
    b"NA_complex_",
    b"function",
    b"while",
    b"repeat",
    b"for",
    b"if",
    b"in",
    b"else",
    b"next",
    b"break",
];

/// `isValidName(name)`: whether the native string `name` is a syntactic
/// name (a letter, or a dot not followed by a digit, then letters, digits,
/// dots and underscores, by the C library's classes) and not a reserved
/// word. An invalid multibyte string before the first character that
/// disqualifies it is `Mbrtowc`'s error.
// R: gram.y:isValidName
pub fn is_valid_name(cx: &mut Cx, name: &[u8]) -> RResult<bool> {
    use robigo_text::tre::Class;
    let loc = crate::text::ctype(cx.locale());
    if loc.mbcs() {
        let mut n = name.len();
        let (used, wc) = r_mbrtowc(cx, name, 0, n)?;
        let mut p = used;
        n -= used;
        if used == 0 {
            return Ok(false);
        }
        if wc != b'.' as i32 && !isw(Class::Alpha, wc as u32, loc) {
            return Ok(false);
        }
        // we don't care about other than ASCII digits
        if wc == b'.' as i32 && name.get(p).is_some_and(u8::is_ascii_digit) {
            return Ok(false);
        }
        loop {
            let (used, wc) = r_mbrtowc(cx, name, p, n)?;
            if used == 0 {
                break;
            }
            if !(isw(Class::Alnum, wc as u32, loc) || wc == b'.' as i32 || wc == b'_' as i32) {
                break;
            }
            p += used;
            n -= used;
        }
        if p != name.len() {
            return Ok(false);
        }
    } else {
        let c = name.first().copied().unwrap_or(0);
        if c != b'.' && !isw(Class::Alpha, c as u32, loc) {
            return Ok(false);
        }
        if c == b'.' && name.get(1).is_some_and(u8::is_ascii_digit) {
            return Ok(false);
        }
        if !name[1..].iter().all(|&c| isw(Class::Alnum, c as u32, loc) || c == b'.' || c == b'_') {
            return Ok(false);
        }
    }
    Ok(name == b"..." || !RESERVED.contains(&name))
}

/// `.Internal(make.names(names, allow_))`: each string made a syntactic
/// name: "X" before one that starts with neither a letter nor a dot (or
/// with a dot and a digit), every character but letters, digits, dots
/// (and with `allow_`, underscores) replaced by a dot, and a dot after a
/// reserved word.
// R: character.c:do_makenames
pub fn do_make_names(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    use robigo_text::tre::Class;
    crate::row!("make.names", true).check_arity(cx, call, args)?;
    let names = args.get(cx, 0);
    if names.sexptype() != STRSXP {
        return Err(cx.error("non-character names"));
    }
    if names.is_package_altrep() {
        return Err(not_yet(cx, Some(call), "make.names() of a package ALTREP vector"));
    }
    let n = names.len().unwrap();
    let allow = as_logical(cx, args.get(cx, 1));
    if allow == NA_INTEGER {
        return Err(cx.error("invalid 'allow_' value"));
    }
    let allow = allow != 0;
    let loc = crate::text::ctype(cx.locale());
    let mut names: Vec<Vec<u8>> = Vec::with_capacity(n);
    for i in 0..n {
        let this = native_bytes(cx, args.root(0), i)?;
        // need to prefix names not beginning with alpha or ., as well as .
        // followed by a number
        let need_prefix = if loc.mbcs() && !this.is_empty() {
            let (used, wc) = r_mbrtowc(cx, &this, 0, 6)?;
            if wc == b'.' as i32 {
                this.len() > used && {
                    let (_, wc) = r_mbrtowc(cx, &this, used, 6)?;
                    isw(Class::Digit, wc as u32, loc)
                }
            } else {
                !isw(Class::Alpha, wc as u32, loc)
            }
        } else if this.first() == Some(&b'.') {
            this.get(1).is_some_and(u8::is_ascii_digit)
        } else {
            !isw(Class::Alpha, this.first().copied().unwrap_or(0) as u32, loc)
        };
        let mut tmp = Vec::with_capacity(this.len() + 2);
        if need_prefix {
            tmp.push(b'X');
        }
        tmp.extend_from_slice(&this);
        let keep = |wc: u32| wc == b'.' as u32 || (allow && wc == b'_' as u32) || isw(Class::Alnum, wc, loc);
        if loc.mbcs() {
            // this cannot lengthen the string
            let Some(wstr) = mbstowcs(&tmp) else {
                return Err(cx.error(&format!("invalid multibyte string {}", i + 1)));
            };
            tmp.clear();
            for wc in wstr {
                wcrtomb_utf8(if keep(wc) { wc } else { b'.' as u32 }, &mut tmp);
            }
        } else {
            for b in tmp.iter_mut() {
                if !keep(*b as u32) {
                    *b = b'.';
                }
            }
        }
        // do we have a reserved word? If so the name is invalid
        if !is_valid_name(cx, &tmp)? {
            tmp.push(b'.');
        }
        names.push(tmp);
    }
    cx.build(out, |cx| {
        let mut ans = cx.alloc_vector::<Char<'_>>(n)?;
        for (i, s) in names.iter().enumerate() {
            ans.set(i, cx.mk_char(s, Encoding::Native)?);
        }
        Ok(ans.finish().into())
    })
}

/// One string of `tolower`/`toupper`: what the C library's `towlower` or
/// `towupper` makes of each character.
enum Cased {
    Na,
    Text(Vec<u8>, Encoding),
}

/// `.Internal(tolower(x))` (CODE 0) and `.Internal(toupper(x))` (CODE 1):
/// each string with its characters mapped by the C library's `towlower`
/// or `towupper`; a UTF-8 (or latin1, in a locale that is not latin1)
/// string comes back UTF-8, another native; the attributes are kept.
// R: character.c:do_tolower
pub fn do_tolower<const CODE: i32>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    use robigo_text::tre::{towlower, towupper};
    let row = if CODE == 1 { crate::row!("toupper", true) } else { crate::row!("tolower", true) };
    row.check_arity(cx, call, args)?;
    let x = args.root(0);
    // coercion is done in wrapper
    if x.bind(cx).sexptype() != STRSXP {
        return Err(cx.error("non-character argument"));
    }
    if x.bind(cx).is_package_altrep() {
        return Err(not_yet(cx, Some(call), "tolower/toupper of a package ALTREP vector"));
    }
    let n = x.bind(cx).len().unwrap();
    let rloc = cx.locale();
    let loc = crate::text::ctype(rloc);
    let map = |wc: u32| if CODE == 1 { towupper(wc, loc) } else { towlower(wc, loc) };
    // IS_UTF8, or IS_LATIN1 in a locale that is not latin1 (none here is)
    let use_utf8 = (0..n).any(|i| {
        let e = x.bind(cx).as_vector::<Char<'_>>().unwrap().get(i).enc();
        matches!(e, Encoding::Utf8 | Encoding::Latin1)
    });
    let mut ys: Vec<Cased> = Vec::with_capacity(n);
    for i in 0..n {
        let c = x.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
        if c == cx.na_string() {
            ys.push(Cased::Na);
            continue;
        }
        if !(loc.mbcs() || use_utf8) {
            let mut xi = native_bytes(cx, x, i)?;
            for b in xi.iter_mut() {
                *b = map(*b as u32) as u8;
            }
            ys.push(Cased::Text(xi, Encoding::Native));
            continue;
        }
        let enc = c.enc();
        if use_utf8 && matches!(enc, Encoding::Utf8 | Encoding::Latin1) {
            // (latin1 by translateCharUTF8, in case it is really in CP1252)
            let xi = match crate::text::to_utf8(crate::coerce::text(c), rloc) {
                Ok(b) => b.into_owned(),
                Err(_) => return Err(cx.error("translating strings with \"bytes\" encoding is not allowed")),
            };
            let mut cbuf = Vec::with_capacity(xi.len());
            for wc in utf8towcs(cx, &xi)? {
                wcrtomb_utf8(map(wc), &mut cbuf);
            }
            ys.push(Cased::Text(cbuf, Encoding::Utf8));
        } else {
            let xi = native_bytes(cx, x, i)?;
            if !loc.mbcs() && !xi.is_ascii() {
                return Err(not_yet(cx, None, "tolower/toupper of a native non-ASCII string in a single-byte locale"));
            }
            let Some(wc) = mbstowcs(&xi) else {
                return Err(cx.error(&format!("invalid multibyte string {}", i + 1)));
            };
            let mut cbuf = Vec::with_capacity(xi.len());
            for w in wc {
                wcrtomb_utf8(map(w), &mut cbuf);
            }
            // markKnown(cbuf, el): the string was native
            ys.push(Cased::Text(cbuf, Encoding::Native));
        }
    }
    cx.build(out, |cx| {
        let mut y = cx.alloc_vector::<Char<'_>>(n)?;
        for (i, s) in ys.iter().enumerate() {
            y.set(
                i,
                match s {
                    Cased::Na => cx.na_string(),
                    Cased::Text(b, e) => cx.mk_char(b, *e)?,
                },
            );
        }
        // SHALLOW_DUPLICATE_ATTRIB(y, x): this copies the class, if any
        let mut a = Attrs::new();
        a.copy_all(x.bind(cx));
        a.apply(cx, &mut y)?;
        Ok(y.finish().into())
    })
}

/// One element of a `chartr` specification (`tr_spec`/`wtr_spec`): a
/// character, or the range `first-last`; a character is the range of one.
#[derive(Clone, Copy)]
struct TrRange {
    first: u32,
    last: u32,
}

/// `tr_build_spec(s, trs)` / `wtr_build_spec(s, trs)`: `s` as characters
/// and ranges (`a-z`), the decreasing range's error raised as each is
/// read. `fmt` writes a character into the message (`%c` / `%lc`).
// R: character.c:wtr_build_spec, character.c:tr_build_spec
fn tr_build_spec(cx: &mut Cx, s: &[u32], fmt: impl Fn(u32, &mut Vec<u8>)) -> RResult<Vec<TrRange>> {
    let len = s.len();
    let mut out = Vec::with_capacity(len);
    let mut i = 0;
    while i + 2 < len {
        if s[i + 1] == b'-' as u32 {
            if s[i] > s[i + 2] {
                let mut m = b"decreasing range specification ('".to_vec();
                fmt(s[i], &mut m);
                m.push(b'-');
                fmt(s[i + 2], &mut m);
                m.extend_from_slice(b"')");
                return Err(cx.error_bytes(&m));
            }
            out.push(TrRange { first: s[i], last: s[i + 2] });
            i += 3;
        } else {
            out.push(TrRange { first: s[i], last: s[i] });
            i += 1;
        }
    }
    for &c in &s[i..] {
        out.push(TrRange { first: c, last: c });
    }
    Ok(out)
}

/// The characters a specification stands for, in order
/// (`tr_get_next_char_from_spec` until it gives `'\0'`).
// R: character.c:wtr_get_next_char_from_spec
fn tr_chars(spec: &[TrRange]) -> impl Iterator<Item = u32> + '_ {
    spec.iter().flat_map(|r| r.first..=r.last)
}

/// `utf8towcs(wc, s, nc + 1)`: R's own UTF-8 decoder over the whole
/// string, its error naming the string.
// R: util.c:utf8towcs
fn utf8towcs(cx: &mut Cx, s: &[u8]) -> RResult<Vec<u32>> {
    let mut out = Vec::with_capacity(s.len());
    let mut t = 0;
    loop {
        let (m, wc) = robigo_text::utf8toucs(s, t);
        if m < 0 {
            let mut msg = b"invalid input '".to_vec();
            msg.extend_from_slice(s);
            msg.extend_from_slice(b"' in 'utf8towcs'");
            return Err(cx.error_bytes(&msg));
        }
        if m == 0 {
            break;
        }
        // (a 4-byte form is one wchar_t on Linux: no surrogate comes out)
        out.push(wc as u32);
        t += m as usize;
    }
    Ok(out)
}

/// `old` or `new` of `chartr` as wide characters: UTF-8 by R's decoder
/// when wide characters are used and it is marked UTF-8 (or latin1,
/// translated to UTF-8 first), else translated to native and decoded by
/// the C library's `mbstowcs` (`what` names it in the error).
fn chartr_wide_arg(cx: &mut Cx, c: Root<'_>, use_wc: bool, what: &str) -> RResult<Vec<u32>> {
    let (enc, bytes) = {
        let ch = c.bind(cx).as_vector::<Char<'_>>().unwrap().get(0);
        (ch.enc(), ch.bytes().to_vec())
    };
    if use_wc && enc == Encoding::Utf8 {
        return utf8towcs(cx, &bytes);
    }
    if use_wc && enc == Encoding::Latin1 {
        let s = Str::new(&bytes, crate::coerce::text_enc(enc));
        let u = crate::text::to_utf8(s, cx.locale()).map(|b| b.into_owned());
        let u = u.map_err(|e| cx.error(&e.to_string()))?;
        return utf8towcs(cx, &u);
    }
    let s = native_bytes(cx, c, 0)?;
    if !crate::text::ctype(cx.locale()).mbcs() && !s.is_ascii() {
        return Err(not_yet(cx, None, "chartr() of a native non-ASCII string in a single-byte locale"));
    }
    mbstowcs(&s).ok_or_else(|| cx.error(&format!("invalid multibyte string '{what}'")))
}

/// One result string of `chartr`.
enum Tr {
    Na,
    Text(Vec<u8>, Encoding),
}

/// `.Internal(chartr(old, new, x))`: each string of `x` with the
/// characters of `old` (with ranges `a-z`) replaced by those of `new` at
/// the same place; a character `old` names twice maps as its last place
/// says. Wide characters in a multibyte locale or when a string is
/// marked; the attributes of `x` are kept.
// R: character.c:do_chartr
pub fn do_chartr(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("chartr", true).check_arity(cx, call, args)?;
    let (old, new, x) = (args.root(0), args.root(1), args.root(2));
    for (a, name) in [(old, "old"), (new, "new")] {
        let v = a.bind(cx);
        if v.sexptype() == STRSXP && v.is_package_altrep() {
            return Err(not_yet(cx, Some(call), "chartr() of a package ALTREP vector"));
        }
        let bad = v.sexptype() != STRSXP || v.len() == Some(0) || {
            let c = v.as_vector::<Char<'_>>().unwrap().get(0);
            c == cx.na_string()
        };
        if bad {
            return Err(cx.error(&format!("invalid '{name}' argument")));
        }
        if a.bind(cx).len().unwrap() > 1 {
            cx.warning(&format!("argument '{name}' has length > 1 and only the first element will be used"))?;
        }
    }
    if x.bind(cx).sexptype() != STRSXP {
        return Err(cx.error("invalid 'x' argument"));
    }
    if x.bind(cx).is_package_altrep() {
        return Err(not_yet(cx, Some(call), "chartr() of a package ALTREP vector"));
    }
    let n = x.bind(cx).len().unwrap();
    let rloc = cx.locale();
    let mbcs = crate::text::ctype(rloc).mbcs();
    // marked strings are done in Unicode (IS_UTF8, or IS_LATIN1 in a
    // locale that is not latin1: none here is)
    let marked = |e: Encoding| matches!(e, Encoding::Utf8 | Encoding::Latin1);
    let use_wc = {
        let enc0 = |r: Root<'_>, cx: &Cx| r.bind(cx).as_vector::<Char<'_>>().unwrap().get(0).enc();
        (0..n).any(|i| marked(x.bind(cx).as_vector::<Char<'_>>().unwrap().get(i).enc()))
            || marked(enc0(old, cx))
            || marked(enc0(new, cx))
    };
    let mut ys: Vec<Tr> = Vec::with_capacity(n);
    if mbcs || use_wc {
        let wfmt = |c: u32, m: &mut Vec<u8>| wcrtomb_utf8(c, m);
        let wc = chartr_wide_arg(cx, old, use_wc, "old")?;
        let trs_old = tr_build_spec(cx, &wc, wfmt)?;
        let wc = chartr_wide_arg(cx, new, use_wc, "new")?;
        let trs_new = tr_build_spec(cx, &wc, wfmt)?;
        let mut newc = tr_chars(&trs_new);
        let mut xtable: Vec<(u32, u32)> = Vec::new();
        for c_old in tr_chars(&trs_old) {
            match newc.next() {
                Some(c_new) => xtable.push((c_old, c_new)),
                None => return Err(cx.error("'old' is longer than 'new'")),
            }
        }
        // ISORT (stable) then COMPRESS: of equal c_old the last one stays
        xtable.sort_by_key(|&(o, _)| o);
        let mut tbl: Vec<(u32, u32)> = Vec::with_capacity(xtable.len());
        for e in xtable {
            match tbl.last_mut() {
                Some(l) if l.0 == e.0 => *l = e,
                _ => tbl.push(e),
            }
        }
        for i in 0..n {
            let (el_na, enc, bytes) = {
                let el = x.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
                (el == cx.na_string(), el.enc(), el.bytes().to_vec())
            };
            if el_na {
                ys.push(Tr::Na);
                continue;
            }
            let utf8 = use_wc && enc == Encoding::Utf8;
            let mut wc = if utf8 {
                utf8towcs(cx, &bytes)?
            } else {
                let xi = native_bytes(cx, x, i)?;
                if !mbcs && !xi.is_ascii() {
                    return Err(not_yet(cx, None, "chartr() of a native non-ASCII string in a single-byte locale"));
                }
                match mbstowcs(&xi) {
                    Some(w) => w,
                    None => return Err(cx.error(&format!("invalid input multibyte string {}", i + 1))),
                }
            };
            for c in wc.iter_mut() {
                if let Ok(k) = tbl.binary_search_by_key(c, |&(o, _)| o) {
                    *c = tbl[k].1;
                }
            }
            let mut cbuf = Vec::with_capacity(bytes.len());
            for &c in &wc {
                wcrtomb_utf8(c, &mut cbuf);
            }
            if !utf8 && !mbcs && !cbuf.is_ascii() {
                return Err(not_yet(cx, None, "chartr() to a non-ASCII character in a single-byte locale"));
            }
            // mkCharCE(cbuf, CE_UTF8), or markKnown(cbuf, el): a marked
            // string comes back UTF-8 in a UTF-8 locale
            let e = if utf8 || (marked(enc) && rloc.utf8) { Encoding::Utf8 } else { Encoding::Native };
            ys.push(Tr::Text(cbuf, e));
        }
    } else {
        let bfmt = |c: u32, m: &mut Vec<u8>| m.push(c as u8);
        let mut xtable: [u8; 256] = std::array::from_fn(|i| i as u8);
        let o = native_bytes(cx, old, 0)?;
        let o: Vec<u32> = o.iter().map(|&b| b as u32).collect();
        let trs_old = tr_build_spec(cx, &o, bfmt)?;
        let nw = native_bytes(cx, new, 0)?;
        let nw: Vec<u32> = nw.iter().map(|&b| b as u32).collect();
        let trs_new = tr_build_spec(cx, &nw, bfmt)?;
        let mut newc = tr_chars(&trs_new);
        for c_old in tr_chars(&trs_old) {
            match newc.next() {
                Some(c_new) => xtable[c_old as usize] = c_new as u8,
                None => return Err(cx.error("'old' is longer than 'new'")),
            }
        }
        for i in 0..n {
            if x.bind(cx).as_vector::<Char<'_>>().unwrap().get(i) == cx.na_string() {
                ys.push(Tr::Na);
                continue;
            }
            let mut xi = native_bytes(cx, x, i)?;
            for b in xi.iter_mut() {
                *b = xtable[*b as usize];
            }
            // markKnown(cbuf, el): no string is marked on this path
            ys.push(Tr::Text(xi, Encoding::Native));
        }
    }
    cx.build(out, |cx| {
        let mut y = cx.alloc_vector::<Char<'_>>(n)?;
        for (i, s) in ys.iter().enumerate() {
            y.set(
                i,
                match s {
                    Tr::Na => cx.na_string(),
                    Tr::Text(b, e) => cx.mk_char(b, *e)?,
                },
            );
        }
        // SHALLOW_DUPLICATE_ATTRIB(y, x): this copies the class, if any
        let mut a = Attrs::new();
        a.copy_all(x.bind(cx));
        a.apply(cx, &mut y)?;
        Ok(y.finish().into())
    })
}
