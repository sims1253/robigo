//! paste.c's `.Internal(paste(list(...), sep, collapse, recycle0))` and
//! `.Internal(paste0(list(...), collapse, recycle0))`: `do_paste`, with GNU
//! R's encoding marks. Each result is built in one reused buffer and
//! interned once (docs/g3.md §7).
//!
//! GNU R stores each coerced element back into the argument list
//! (`SET_VECTOR_ELT(x, j, ...)`); here they are held in roots instead, so
//! the list an R caller passed is never changed.

use crate::coerce::{Strings, Uncoercible};
use crate::registry;
use crate::text::{to_native, to_utf8};
use crate::util::{as_logical_n, not_yet};
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
    let def = registry::by_name(if use_sep { "paste" } else { "paste0" }).expect("robigo: paste in the registry");
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
            recycle0 = as_bool2(cx, call, args.root(3))?;
        }
        (Some(args.root(1)), args.root(2))
    } else {
        if correct {
            recycle0 = as_bool2(cx, call, args.root(2))?;
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
                elts.push(Some(r));
                len
            } else {
                match Strings::of(cx, elt(cx, x, j)).map(|v| v.len()) {
                    Ok(n) => n,
                    Err(e @ (Uncoercible::Type(_) | Uncoercible::Env)) => return Err(cx.error(&e.message())),
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

/// `asBool2(x, call)`: a logical of length one that is not NA.
// R: coerce.c:asBool2, coerce.c:asLogical2
fn as_bool2(cx: &mut Cx, call: Root<'_>, x: Root<'_>) -> RResult<bool> {
    let (v, n) = as_logical_n(cx, x.bind(cx));
    if crate::util::is_vector_atomic(x.bind(cx)) && n > 1 {
        let msg = format!("'length = {n}' in coercion to 'logical(1)'");
        return Err(cx.error_call(Some(call), &msg));
    }
    if v == crate::kernel::logic::NA {
        return Err(cx.error_call(Some(call), "NA in coercion to boolean"));
    }
    Ok(v != 0)
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
    let call = s.capture(cx, |cx, out| {
        cx.build(out, |cx| {
            let f = cx.install(b"as.character")?;
            let a = cx.cons(xj.bind(cx), cx.nil())?;
            cx.lcons(f.into(), a)
        })
    })?;
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
    // the elements (text and mark) GNU R stores in `ans`
    let mut ans: Vec<(Vec<u8>, Enc)> = Vec::with_capacity(maxlen);
    let mut buf = Vec::new();
    for i in 0..maxlen {
        let (mut any_known, mut all_known, mut use_utf8, mut use_bytes) = (false, true, false, false);
        if nx > 1 {
            all_known = sep_known || sep_ascii;
            any_known = sep_known;
            use_utf8 = sep_utf8;
            use_bytes = sep_bytes;
        }
        for s in sv {
            let k = s.len();
            if k > 0 {
                let cs = s.get_or_na(i % k);
                use_utf8 |= cs.enc() == Enc::Utf8;
                use_bytes |= cs.enc() == Enc::Bytes;
            }
        }
        if use_bytes {
            use_utf8 = false;
        }
        buf.clear();
        for (j, s) in sv.iter().enumerate() {
            let k = s.len();
            if k > 0 {
                let cs = s.get_or_na(i % k);
                if use_utf8 {
                    buf.extend_from_slice(&to_utf8(cs, loc).expect("robigo: no bytes when use_UTF8"));
                } else {
                    if use_bytes {
                        buf.extend_from_slice(cs.bytes());
                    } else {
                        buf.extend_from_slice(&to_native(cs, loc).expect("robigo: no bytes element here"));
                    }
                    all_known = all_known && (cs.is_ascii() || known(cs));
                    any_known = any_known || known(cs);
                }
            }
            if sepw != 0 && j != nx - 1 {
                let sep = sep.unwrap();
                if use_utf8 {
                    buf.extend_from_slice(&to_utf8(sep, loc).expect("robigo: sep checked"));
                } else {
                    buf.extend_from_slice(&csep_native);
                }
            }
        }
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
        let enc = if buf.is_ascii() { Enc::Native } else { enc };
        ans.push((buf.clone(), enc));
    }
    let Some(collapse) = collapse else {
        let mut v = cx.alloc_vector::<Char<'_>>(maxlen)?;
        for (i, (b, e)) in ans.iter().enumerate() {
            v.set(i, cx.mk_char(b, mark(*e))?);
        }
        return Ok(Some(v.finish().into()));
    };
    // collapse
    let sep = crate::coerce::text(collapse);
    let mut use_utf8 = sep.enc() == Enc::Utf8;
    let mut use_bytes = sep.enc() == Enc::Bytes;
    for (b, e) in &ans {
        let el = Str::new(b, *e);
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
    for (i, (b, e)) in ans.iter().enumerate() {
        if i > 0 {
            buf.extend_from_slice(&csep);
        }
        let el = Str::new(b, *e);
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
