//! paste.c's `.Internal(paste(list(...), sep, collapse, recycle0))` and
//! `.Internal(paste0(list(...), collapse, recycle0))`: `do_paste`, with GNU
//! R's encoding marks. Each result is built in one reused buffer and
//! interned once (docs/g3.md §7).
//!
//! GNU R stores each coerced element back into the argument list
//! (`SET_VECTOR_ELT(x, j, ...)`); here they are held in roots instead, so
//! the list an R caller passed is never changed.

use crate::coerce::{Strings, Uncoercible};
use crate::text::{to_native, to_utf8};
use crate::util::not_yet;
use robigo_rtcore::RSym;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Built, Char, Cx, Diag, Encoding, NeedGc, Out, RResult, Ref, Root, View};
use robigo_text::{Enc, Str};

/// `do_paste`: `paste` (`CODE` 0, with `sep`) and `paste0` (1).
// R: paste.c:do_paste
pub fn do_paste<const CODE: u8>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let use_sep = CODE == 0;
    let def = if use_sep { crate::row!("paste") } else { crate::row!("paste0") };
    // the arity, or one less (code byte-compiled before `recycle0`)
    let nargs = args.len();
    let correct = nargs as i32 == def.arity;
    if !correct && def.arity != nargs as i32 + 1 {
        let s = if nargs == 1 { "" } else { "s" };
        let msg = format!("{nargs} argument{s} passed to .Internal({}) which requires {}", def.name, def.arity);
        return Err(cx.error(&msg));
    }
    let x = args.root(0);
    if !matches!(x.bind(cx).sexptype(), VECSXP | EXPRSXP) {
        return Err(cx.error("invalid first argument"));
    }
    // `list(...)`'s elements are read as the builtin's arguments are
    // (`list` leaves one of robigo's compact sequences as it is)
    cx.materialize_nested(x, Some(1))?;
    let Some(nx) = x.bind(cx).std_vector().map(|v| v.len()) else {
        return Err(not_yet(cx, Some(call), "paste() of a package ALTREP list"));
    };
    let mut recycle0 = false;
    let (sep, collapse) = if use_sep {
        let sep = args.get(cx, 1);
        match sep.as_vector::<Char<'_>>().and_then(|v| v.as_slice()).and_then(|s| s.first().copied()) {
            Some(c) if c != cx.na_string() => {
                // translateChar(sep)
                if c.enc() == Encoding::Bytes && !c.is_ascii() {
                    return Err(cx.error("translating strings with \"bytes\" encoding is not allowed"));
                }
            }
            _ => return Err(cx.error("invalid separator")),
        }
        if correct {
            recycle0 = crate::util::as_bool2(cx, call, args.root(3))?;
        }
        (Some(args.root(1)), args.root(2))
    } else {
        if correct {
            recycle0 = crate::util::as_bool2(cx, call, args.root(2))?;
        }
        (None, args.root(1))
    };
    let do_collapse = collapse.bind(cx).sexptype() != NILSXP;
    if do_collapse {
        let c = collapse.bind(cx);
        let ok = c
            .as_vector::<Char<'_>>()
            .and_then(|v| v.as_slice())
            .and_then(|s| s.first().copied())
            .is_some_and(|e| e != cx.na_string());
        if !ok {
            return Err(cx.error("invalid 'collapse' argument"));
        }
    }
    if nx == 0 {
        return zero(cx, do_collapse, out);
    }
    cx.scope(|cx, s| {
        // coerce: an object through as.character() (method dispatch), a
        // symbol to its name, anything else with coerceVector
        let mut elts: Vec<Option<Root<'_>>> = Vec::with_capacity(nx);
        // GNU R stores an object's as.character() value back into the list
        // (SET_VECTOR_ELT(x, j, ...)), whose link keeps a later method from
        // writing it in place; robigo leaves the argument as it is and links
        // the values from a list of its own, made at the first object
        let mut held: Option<Root<'_>> = None;
        let mut maxlen = 0usize;
        let mut has0 = false;
        for j in 0..nx {
            let (is_str, is_obj) = {
                let xj = elt(cx, x, j);
                (xj.sexptype() == STRSXP, xj.is_object())
            };
            let len = if is_str {
                match elt(cx, x, j).std_vector() {
                    Some(v) => v.len(),
                    None => return Err(not_yet(cx, Some(call), "paste() of a package ALTREP character vector")),
                }
            } else if is_obj {
                let xj = s.root(cx, elt(cx, x, j));
                let r = as_character(cx, s, xj, env)?;
                let len = match r.bind(cx).std_vector() {
                    Some(v) if v.sexptype() == STRSXP => v.len(),
                    Some(_) => return Err(non_string(cx, def.name)),
                    None if r.bind(cx).sexptype() == STRSXP => {
                        return Err(not_yet(cx, Some(call), "paste() of a package ALTREP character vector"));
                    }
                    None => return Err(non_string(cx, def.name)),
                };
                let h = match held {
                    Some(h) => h,
                    None => {
                        let h = s.capture(cx, |cx, o| {
                            cx.build(o, |cx| Ok(cx.alloc_vector::<Ref<'_>>(nx)?.finish().into()))
                        })?;
                        held = Some(h);
                        h
                    }
                };
                cx.owned_list(h)?.set(j, r);
                elts.push(Some(r));
                len
            } else {
                match Strings::of(cx, elt(cx, x, j)).map(|v| v.len()) {
                    Ok(n) => n,
                    Err(e @ (Uncoercible::Type(_) | Uncoercible::Env | Uncoercible::Deparse(_))) => {
                        return Err(cx.error(&e.message()));
                    }
                    Err(Uncoercible::NotYet(what)) => return Err(not_yet(cx, Some(call), what)),
                }
            };
            if elts.len() == j {
                elts.push(None);
            }
            if recycle0 && len == 0 {
                has0 = true;
                break;
            }
            maxlen = maxlen.max(len);
        }
        if (recycle0 && has0) || maxlen == 0 {
            return zero(cx, do_collapse, out);
        }
        cx.build_with(out, |cx| {
            let sv: Vec<Strings<'_>> = (0..nx)
                .map(|j| {
                    let e = elts[j].map_or_else(|| elt(cx, x, j), |r| r.bind(cx));
                    Strings::of(cx, e).expect("robigo: checked before")
                })
                .collect();
            Ok(match paste_all(cx, &sv, sep.map(|s| first(cx, s)), do_collapse.then(|| first(cx, collapse)), maxlen)? {
                Some(r) => Built::done(r),
                None => Built::fail(Diag::error("result would exceed 2^31-1 bytes")),
            })
        })
    })
}

/// `.Internal(file.path(list(...), fsep))`: the arguments' elements
/// joined by `fsep`, recycled to the longest; character(0) if any is empty.
/// Each piece is in UTF-8 in a UTF-8 locale (or when one is marked), else
/// native; "bytes" strings are an error.
// R: paste.c:do_filepath
pub fn do_filepath(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("file.path", true).check_arity(cx, call, args)?;
    let x = args.root(0);
    if !matches!(x.bind(cx).sexptype(), VECSXP | EXPRSXP) {
        return Err(cx.error("invalid first argument"));
    }
    // `list(...)`'s elements are read as the builtin's arguments are
    // (`list` leaves one of robigo's compact sequences as it is)
    cx.materialize_nested(x, Some(1))?;
    let nx = x.bind(cx).len().unwrap_or(0);
    let empty = |cx: &mut Cx, out: Out<'_>| cx.build(out, |cx| Ok(cx.alloc_vector::<Char<'_>>(0)?.finish().into()));
    if nx == 0 {
        return empty(cx, out);
    }
    let sep = args.root(1);
    let sep_ok = sep.bind(cx).as_vector::<Char<'_>>().is_some_and(|v| !v.is_empty() && v.get(0) != cx.na_string());
    if !sep_ok {
        return Err(cx.error("invalid separator"));
    }
    cx.scope(|cx, s| {
        // the arguments as strings: an object through as.character(), a
        // symbol its name, anything else coerceVector(xj, STRSXP). GNU R
        // stores an object's value back into the list (SET_VECTOR_ELT),
        // whose link keeps a later method from writing it in place;
        // robigo links the values from a list of its own, as do_paste does
        let mut elts: Vec<Option<Root<'_>>> = Vec::with_capacity(nx);
        let mut held: Option<Root<'_>> = None;
        let mut maxlen = 0usize;
        for j in 0..nx {
            let (is_str, is_obj) = {
                let xj = elt(cx, x, j);
                (xj.sexptype() == STRSXP, xj.is_object())
            };
            let len = if is_str {
                elts.push(None);
                elt(cx, x, j).len().unwrap_or(0)
            } else if is_obj {
                let xj = s.root(cx, elt(cx, x, j));
                let r = as_character(cx, s, xj, env)?;
                if r.bind(cx).sexptype() != STRSXP {
                    return Err(non_string(cx, "file.path"));
                }
                let h = match held {
                    Some(h) => h,
                    None => {
                        let h = s.capture(cx, |cx, o| {
                            cx.build(o, |cx| Ok(cx.alloc_vector::<Ref<'_>>(nx)?.finish().into()))
                        })?;
                        held = Some(h);
                        h
                    }
                };
                cx.owned_list(h)?.set(j, r);
                elts.push(Some(r));
                r.bind(cx).len().unwrap_or(0)
            } else {
                let n = match Strings::of(cx, elt(cx, x, j)).map(|v| v.len()) {
                    Ok(n) => n,
                    Err(e @ (Uncoercible::Type(_) | Uncoercible::Env | Uncoercible::Deparse(_))) => {
                        return Err(cx.error(&e.message()));
                    }
                    Err(Uncoercible::NotYet(what)) => return Err(not_yet(cx, Some(call), what)),
                };
                elts.push(None);
                n
            };
            if len == 0 {
                return empty(cx, out);
            }
            maxlen = maxlen.max(len);
        }
        let loc = cx.locale();
        let utf8_locale = loc.utf8;
        // the pieces' bytes, element by element; whether element i is UTF-8
        let mut joined: Vec<(Vec<u8>, bool)> = Vec::with_capacity(maxlen);
        let bytes_found = {
            let cx = &*cx;
            let sv: Vec<Strings<'_>> = (0..nx)
                .map(|j| {
                    let e = elts[j].map_or_else(|| elt(cx, x, j), |r| r.bind(cx));
                    Strings::of(cx, e).expect("robigo: checked before")
                })
                .collect();
            let bytes_found = sv.iter().any(|v| (0..v.len()).any(|i| v.get(i).is_some_and(|t| t.enc() == Enc::Bytes)));
            let csep = first(cx, sep).bytes().to_vec();
            for i in (0..maxlen).take_while(|_| !bytes_found) {
                // (latin1locale is not modelled)
                let use_utf8 = utf8_locale
                    || sv.iter().any(|v| {
                        let t = v.get_or_na(i % v.len());
                        matches!(t.enc(), Enc::Utf8 | Enc::Latin1)
                    });
                let mut buf = Vec::new();
                for (j, v) in sv.iter().enumerate() {
                    let t = v.get_or_na(i % v.len());
                    // trCharUTF8 / translateCharFP ("bytes" was refused)
                    let piece = if use_utf8 { to_utf8(t, loc) } else { to_native(t, loc) };
                    buf.extend_from_slice(&piece.expect("robigo: no bytes string"));
                    if j != nx - 1 {
                        buf.extend_from_slice(&csep);
                    }
                }
                joined.push((buf, use_utf8));
            }
            bytes_found
        };
        if bytes_found {
            return Err(cx.error("strings with \"bytes\" encoding are not allowed"));
        }
        cx.build(out, |cx| {
            let mut ans = cx.alloc_vector::<Char<'_>>(maxlen)?;
            for (i, (b, u)) in joined.iter().enumerate() {
                ans.set(i, cx.mk_char(b, if *u { Encoding::Utf8 } else { Encoding::Native })?);
            }
            Ok(ans.finish().into())
        })
    })
}

/// GNU R's `zero_return`: `""` when collapsing, else `character(0)`.
fn zero(cx: &mut Cx, collapse: bool, out: Out<'_>) -> RResult<()> {
    cx.build(out, |cx| {
        let mut v = cx.alloc_vector::<Char<'_>>(collapse as usize)?;
        if collapse {
            v.set(0, cx.blank_string());
        }
        Ok(v.finish().into())
    })
}

fn non_string(cx: &mut Cx, name: &str) -> robigo_rtcore::RError {
    cx.error(&format!("non-string argument to .Internal({name})"))
}

/// Element `j` of the list.
fn elt<'g>(cx: &'g Cx, x: Root<'_>, j: usize) -> Ref<'g> {
    match x.bind(cx).view() {
        View::List(v) | View::Expr(v) => v.get(j),
        _ => unreachable!("robigo: checked to be a list"),
    }
}

/// `eval(lang2(R_AsCharacterSymbol, xj), env)`, rooted.
fn as_character<'s>(cx: &mut Cx, s: robigo_rtcore::Scope<'s>, xj: Root<'_>, env: Root<'_>) -> RResult<Root<'s>> {
    let call = s.capture(cx, |cx, out| cx.build(out, |cx| cx.lang2(cx.sym(RSym::AsCharacter).into(), xj.bind(cx))))?;
    s.capture(cx, |cx, out| cx.eval(call, env, out))
}

/// The first element of a character vector.
fn first<'g>(cx: &'g Cx, r: Root<'_>) -> Char<'g> {
    r.bind(cx).as_vector::<Char<'_>>().unwrap().get(0)
}

/// `ENC_KNOWN(s) > 0`: marked latin1 or UTF-8.
fn known(s: Str<'_>) -> bool {
    matches!(s.enc(), Enc::Latin1 | Enc::Utf8)
}

fn mark(enc: Enc) -> Encoding {
    match enc {
        Enc::Native => Encoding::Native,
        Enc::Utf8 => Encoding::Utf8,
        Enc::Latin1 => Encoding::Latin1,
        Enc::Bytes => Encoding::Bytes,
    }
}

/// The pasting and collapsing of `do_paste` (under `&Cx`: nothing here
/// can fail but the allocation). None: a result over `INT_MAX` bytes.
// R: paste.c:do_paste
fn paste_all<'g>(
    cx: &'g Cx,
    sv: &[Strings<'_>],
    sep: Option<Char<'_>>,
    collapse: Option<Char<'_>>,
    maxlen: usize,
) -> Result<Option<Ref<'g>>, NeedGc> {
    let loc = cx.locale();
    let nx = sv.len();
    let sep = sep.map(crate::coerce::text);
    // (translateChar(sep) was checked; a sep needs none in a UTF-8 locale
    // unless latin1, which is translated per use below)
    let csep_native = sep.map(|s| to_native(s, loc).expect("robigo: sep checked")).unwrap_or_default();
    let sepw = if sep.is_some() { csep_native.len() } else { 0 };
    let (sep_ascii, sep_known, sep_utf8, sep_bytes) = match sep {
        Some(s) => (s.is_ascii(), known(s), s.enc() == Enc::Utf8, s.enc() == Enc::Bytes),
        None => (true, false, false, false),
    };
    let known_utf8 = loc.utf8;
    // without collapse, each element is interned from the reused buffer
    // into the result; with it, the elements' texts are kept end to end
    // in `pieces` (their ends and marks in `ans`) for the second pass
    let mut result = if collapse.is_none() { Some(cx.alloc_vector::<Char<'_>>(maxlen)?) } else { None };
    let mut pieces: Vec<u8> = Vec::new();
    let mut ans: Vec<(usize, Enc)> = Vec::with_capacity(if collapse.is_some() { maxlen } else { 0 });
    let mut buf = Vec::new();
    // each argument's piece of element `i` (None: a zero-length argument),
    // read once, with whether it is ASCII (the CHARSXP's mark)
    let mut pcs: Vec<Option<(Str<'_>, bool)>> = Vec::with_capacity(nx);
    for i in 0..maxlen {
        let (mut any_known, mut all_known, mut use_utf8, mut use_bytes) = (false, true, false, false);
        if nx > 1 {
            all_known = sep_known || sep_ascii;
            any_known = sep_known;
            use_utf8 = sep_utf8;
            use_bytes = sep_bytes;
        }
        pcs.clear();
        for s in sv {
            let k = s.len();
            pcs.push((k > 0).then(|| s.get_or_na_ascii(i % k)));
            if let Some(Some((cs, _))) = pcs.last() {
                use_utf8 |= cs.enc() == Enc::Utf8;
                use_bytes |= cs.enc() == Enc::Bytes;
            }
        }
        if use_bytes {
            use_utf8 = false;
        }
        buf.clear();
        // whether every piece and separator added is ASCII, so the result
        // is (an ASCII text is the same in every encoding, and is never
        // ENC_KNOWN: the marks below are unchanged by it)
        let mut ascii = true;
        for (j, p) in pcs.iter().enumerate() {
            if let Some((cs, cs_ascii)) = *p {
                if cs_ascii {
                    buf.extend_from_slice(cs.bytes());
                } else {
                    ascii = false;
                    if use_utf8 {
                        buf.extend_from_slice(&to_utf8(cs, loc).expect("robigo: no bytes when use_UTF8"));
                    } else {
                        if use_bytes {
                            buf.extend_from_slice(cs.bytes());
                        } else {
                            buf.extend_from_slice(&to_native(cs, loc).expect("robigo: no bytes element here"));
                        }
                        all_known = all_known && known(cs);
                        any_known = any_known || known(cs);
                    }
                }
            }
            if sepw != 0 && j != nx - 1 {
                let sep = sep.unwrap();
                ascii &= sep_ascii;
                if use_utf8 {
                    buf.extend_from_slice(&to_utf8(sep, loc).expect("robigo: sep checked"));
                } else {
                    buf.extend_from_slice(&csep_native);
                }
            }
        }
        // (a translation of a non-ASCII piece may come out ASCII: an
        // escape in a non-UTF-8 locale)
        let ascii = ascii || buf.is_ascii();
        if buf.len() > i32::MAX as usize {
            return Ok(None);
        }
        let enc = if use_utf8 {
            Enc::Utf8
        } else if use_bytes {
            Enc::Bytes
        } else if any_known && all_known && known_utf8 {
            Enc::Utf8
        } else {
            Enc::Native
        };
        // mkCharCE: ASCII carries no mark
        let enc = if ascii { Enc::Native } else { enc };
        match result.as_mut() {
            // (the pieces are CHARSXPs' and coerced texts: no NUL)
            Some(v) => v.set(i, cx.mk_char_joined(&buf, ascii, mark(enc))?),
            None => {
                pieces.extend_from_slice(&buf);
                ans.push((pieces.len(), enc));
            }
        }
    }
    let Some(collapse) = collapse else {
        return Ok(Some(result.unwrap().finish().into()));
    };
    let elt = |i: usize| -> (&[u8], Enc) {
        let start = if i == 0 { 0 } else { ans[i - 1].0 };
        (&pieces[start..ans[i].0], ans[i].1)
    };
    // collapse
    let sep = crate::coerce::text(collapse);
    let mut use_utf8 = sep.enc() == Enc::Utf8;
    let mut use_bytes = sep.enc() == Enc::Bytes;
    for i in 0..ans.len() {
        let (b, e) = elt(i);
        let el = Str::new(b, e);
        use_utf8 |= el.enc() == Enc::Utf8;
        use_bytes |= el.enc() == Enc::Bytes;
    }
    let csep: Vec<u8> = if use_bytes {
        use_utf8 = false;
        sep.bytes().to_vec()
    } else if use_utf8 {
        to_utf8(sep, loc).expect("robigo: not bytes").into_owned()
    } else {
        to_native(sep, loc).expect("robigo: not bytes").into_owned()
    };
    let mut any_known = known(sep);
    let mut all_known = any_known || sep.is_ascii();
    buf.clear();
    for i in 0..ans.len() {
        let (b, e) = elt(i);
        if i > 0 {
            buf.extend_from_slice(&csep);
        }
        let el = Str::new(b, e);
        if use_utf8 {
            buf.extend_from_slice(&to_utf8(el, loc).expect("robigo: not bytes"));
        } else {
            buf.extend_from_slice(b);
        }
        all_known = all_known && (el.is_ascii() || known(el));
        any_known = any_known || known(el);
        if buf.len() > i32::MAX as usize {
            return Ok(None);
        }
    }
    let enc = if use_utf8 {
        Enc::Utf8
    } else if use_bytes {
        Enc::Bytes
    } else if any_known && all_known && known_utf8 {
        Enc::Utf8
    } else {
        Enc::Native
    };
    let enc = if buf.is_ascii() { Enc::Native } else { enc };
    let mut v = cx.alloc_vector::<Char<'_>>(1)?;
    v.set(0, cx.mk_char(&buf, mark(enc))?);
    Ok(Some(v.finish().into()))
}
