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
        Err(e @ (Uncoercible::Type(_) | Uncoercible::Env)) => return Err(cx.error(&e.message())),
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

// ---- substr, startsWith (B2b) ------------------------------------------------

/// `.Internal(substr(x, start, stop))`: characters `start..=stop` of each
/// string (counted in UTF-8 characters for UTF-8 and native strings in a
/// UTF-8 locale, else bytes), the attributes kept.
// R: character.c:do_substr, substr
pub fn do_substr(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("substr", true).unwrap().check_arity(cx, call, args)?;
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
    let r = cx.build_with(out, |cx| {
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
    });
    r
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
    let name = if CODE == 0 { "startsWith" } else { "endsWith" };
    crate::registry::lookup(name, true).unwrap().check_arity(cx, call, args)?;
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
                crate::text::to_utf8(crate::coerce::text(c), loc).map(|b| b.into_owned()).unwrap_or_else(|_| c.bytes().to_vec())
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
