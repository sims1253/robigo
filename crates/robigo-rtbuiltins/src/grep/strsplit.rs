//! `.Internal(strsplit(x, split, fixed, perl, useBytes))`.

use super::{
    Elt, Fail, Pcre, Report, Subj, bytes_new, have, input, mark_bytes_old, mark_known, only_ascii, pattern,
    pcre_exec_error, strings, tr_char, tre_comp_native, tre_exec, tre_fail, utf8clen, wide_input, wide_utf8, wtrans,
};
use crate::attrib::{self, Attr, Attrs};
use crate::util::as_bool2;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Built, Char, Cx, Encoding, Out, RResult, RSym, Ref, Root};
use robigo_text::tre::{self, Match, Regex};

/// The result for one string: NA (`ScalarString(NA_STRING)`), or its
/// pieces.
type Pieces = Option<Vec<Elt>>;

/// How the strings are matched (do_strsplit's flags).
#[derive(Clone, Copy)]
struct Mode {
    fixed: bool,
    perl: bool,
    use_bytes: bool,
    use_utf8: bool,
    have_bytes_input: bool,
    loc: robigo_rtcore::Locale,
}

/// The mark of a piece of `x`'s string `xi` cut by the fixed, PCRE or
/// byte-ERE paths.
fn piece_mark(m: &Mode, xi: Char<'_>) -> Encoding {
    if m.use_bytes {
        bytes_new(m.have_bytes_input)
    } else if m.use_utf8 {
        Encoding::Utf8
    } else {
        mark_known(xi, m.loc)
    }
}

// R: grep.c:do_strsplit
pub fn do_strsplit(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("strsplit", true).check_arity(cx, call, args)?;
    let x = args.root(0);
    let tok = args.root(1);
    let fixed = as_bool2(cx, call, args.root(2))?;
    let mut perl = as_bool2(cx, call, args.root(3))?;
    let use_bytes = as_bool2(cx, call, args.root(4))?;
    if fixed && perl {
        cx.warning("argument 'perl = TRUE' will be ignored")?;
        perl = false;
    }
    if x.bind(cx).sexptype() != STRSXP || tok.bind(cx).sexptype() != STRSXP {
        return Err(cx.error("non-character argument"));
    }
    let loc = cx.locale();
    let mut rep = Report::default();
    let res = {
        let cx: &Cx = cx;
        let xv = strings(x.bind(cx));
        let tv = strings(tok.bind(cx));
        // treat split = NULL as split = ""
        let blank = [cx.blank_string()];
        let toks: Vec<Char<'_>> =
            if tv.is_empty() { blank.to_vec() } else { (0..tv.len()).map(|i| tv.get(i)).collect() };
        let xs: Vec<Char<'_>> = (0..xv.len()).map(|i| xv.get(i)).collect();
        split_all(cx, &xs, &toks, fixed, perl, use_bytes, loc, &mut rep)
    };
    let pieces = rep.finish(cx, call, res)?;
    cx.build_with(out, |cx| {
        let mut ans = cx.alloc_vector::<Ref<'_>>(pieces.len())?;
        for (i, p) in pieces.iter().enumerate() {
            let t = match p {
                None => {
                    let mut t = cx.alloc_vector::<Char<'_>>(1)?;
                    t.set(0, cx.na_string());
                    t.finish()
                }
                Some(v) => {
                    let mut t = cx.alloc_vector::<Char<'_>>(v.len())?;
                    for (j, e) in v.iter().enumerate() {
                        t.set(j, e.make(cx)?);
                    }
                    t.finish()
                }
            };
            ans.set(i, t.into());
        }
        let xv = x.bind(cx);
        if let Some(nm) = attrib::names(xv) {
            let mut a = Attrs::new();
            a.install(Attr { tag: cx.sym(RSym::Names), value: nm });
            a.apply(cx, &mut ans)?;
        }
        Ok(Built::done(ans.finish()))
    })
}

/// Phase 2: every string split.
#[allow(clippy::too_many_arguments)]
fn split_all(
    cx: &Cx,
    xs: &[Char<'_>],
    toks: &[Char<'_>],
    fixed: bool,
    perl: bool,
    use_bytes: bool,
    loc: robigo_rtcore::Locale,
    rep: &mut Report,
) -> Result<Vec<Pieces>, Fail> {
    let (len, tlen) = (xs.len(), toks.len());
    let have_bytes_input = have(toks, Encoding::Bytes) || have(xs, Encoding::Bytes);
    let mut use_bytes = use_bytes;
    if !use_bytes {
        use_bytes = only_ascii(cx, toks) && only_ascii(cx, xs);
    }
    if !use_bytes {
        use_bytes = have_bytes_input;
    }
    let mut use_utf8 = false;
    if !use_bytes {
        // use_UTF8 means use wchar_t* for the TRE engine
        if !fixed && loc.utf8 {
            use_utf8 = true;
        }
        if !use_utf8 {
            use_utf8 = have(toks, Encoding::Utf8) || have(xs, Encoding::Utf8);
        }
        // (not a latin1 locale)
        if !use_utf8 {
            use_utf8 = have(toks, Encoding::Latin1) || have(xs, Encoding::Latin1);
        }
    }
    let m = Mode { fixed, perl, use_bytes, use_utf8, have_bytes_input, loc };
    let mut ans: Vec<Pieces> = vec![None; len];
    // group by token for efficiency with PCRE/TRE versions
    for (itok, &this) in toks.iter().enumerate() {
        let elts = (itok..len).step_by(tlen);
        if this == cx.na_string() {
            // NA token doesn't split
            for i in elts {
                ans[i] = Some(vec![mark_bytes_old(cx, xs[i], m.use_bytes, m.have_bytes_input)]);
            }
        } else if this.bytes().is_empty() {
            for i in elts {
                ans[i] = split_chars(cx, &m, xs[i], i, rep);
            }
        } else if m.fixed {
            split_fixed(cx, &m, this, itok, xs, elts, &mut ans, rep)?;
        } else if m.perl {
            split_perl(cx, &m, this, itok, xs, elts, &mut ans, rep)?;
        } else if !m.use_bytes && m.use_utf8 {
            split_wide(cx, &m, this, itok, xs, elts, &mut ans, rep)?;
        } else {
            split_bytes(cx, &m, this, itok, xs, elts, &mut ans, rep)?;
        }
    }
    Ok(ans)
}

/// `split = ""`: the string's characters (bytes when `useBytes`, or when
/// it is ASCII or in a single-byte locale and not UTF-8).
fn split_chars(cx: &Cx, m: &Mode, xi: Char<'_>, i: usize, rep: &mut Report) -> Pieces {
    if xi == cx.na_string() {
        return None;
    }
    let buf = input(xi, i, m.use_bytes, m.use_utf8, m.loc, rep)?;
    let mut t = Vec::with_capacity(buf.len());
    if !m.use_bytes && (m.use_utf8 || m.loc.utf8) && !xi.is_ascii() {
        // split into individual characters (not bytes); in a UTF-8 locale
        // mbrtowc's characters are utf8clen's (the string is valid)
        let enc = if m.use_utf8 { Encoding::Utf8 } else { mark_known(xi, m.loc) };
        let mut p = 0;
        while p < buf.len() {
            let used = utf8clen(buf[p]).min(buf.len() - p);
            t.push(Elt::Str(buf[p..p + used].to_vec(), enc));
            p += used;
        }
    } else {
        let enc = if m.use_bytes { bytes_new(m.have_bytes_input) } else { mark_known(xi, m.loc) };
        t.extend(buf.iter().map(|&b| Elt::Str(vec![b], enc)));
    }
    Some(t)
}

fn split_string<'a>(
    m: &Mode,
    this: Char<'a>,
    itok: usize,
    rep: &mut Report,
) -> Result<std::borrow::Cow<'a, [u8]>, Fail> {
    pattern(this, || format!("'split' string {}", itok + 1), m.use_bytes, m.use_utf8, m.loc, rep)
}

#[allow(clippy::too_many_arguments)]
fn split_fixed(
    cx: &Cx,
    m: &Mode,
    this: Char<'_>,
    itok: usize,
    xs: &[Char<'_>],
    elts: impl Iterator<Item = usize>,
    ans: &mut [Pieces],
    rep: &mut Report,
) -> Result<(), Fail> {
    let split = split_string(m, this, itok, rep)?;
    let slen = split.len();
    for i in elts {
        let xi = xs[i];
        if xi == cx.na_string() {
            continue;
        }
        let Some(buf) = input(xi, i, m.use_bytes, m.use_utf8, m.loc, rep) else { continue };
        let enc = piece_mark(m, xi);
        // This is UTF-8 safe since it compares whole strings
        let mut t = Vec::new();
        let (mut last, mut p) = (0, 0);
        while p < buf.len() {
            if buf[p..].starts_with(&split) {
                t.push(Elt::Str(buf[last..p].to_vec(), enc));
                p += slen;
                last = p;
            } else {
                p += 1;
            }
        }
        if last < buf.len() {
            t.push(Elt::Str(buf[last..].to_vec(), enc));
        }
        ans[i] = Some(t);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn split_perl(
    cx: &Cx,
    m: &Mode,
    this: Char<'_>,
    itok: usize,
    xs: &[Char<'_>],
    elts: impl Iterator<Item = usize>,
    ans: &mut [Pieces],
    rep: &mut Report,
) -> Result<(), Fail> {
    let split = split_string(m, this, itok, rep)?;
    let re = Pcre::prepare(&split, m.use_utf8, false, m.loc, rep)?;
    for i in elts {
        let xi = xs[i];
        if xi == cx.na_string() {
            continue;
        }
        let Some(buf) = input(xi, i, m.use_bytes, m.use_utf8, m.loc, rep) else { continue };
        let enc = piece_mark(m, xi);
        let mut t = Vec::new();
        let mut p = 0;
        // each match on the rest of the string, without PCRE2_NOTBOL
        while p < buf.len() {
            let rest = &buf[p..];
            match re.find_at(rest, 0) {
                Ok(Some((so, eo))) => {
                    if eo > 0 {
                        // Match was non-empty.
                        t.push(Elt::Str(rest[..so].to_vec(), enc));
                        p += eo;
                    } else {
                        // Match was empty: the next character.
                        // Only use_UTF8 input has been validated as UTF-8.
                        let clen = if m.use_utf8 { utf8clen(rest[0]) } else { 1 };
                        t.push(Elt::Str(rest[..clen.min(rest.len())].to_vec(), enc));
                        p += clen;
                    }
                }
                Ok(None) => break,
                Err(msg) => {
                    rep.warn(pcre_exec_error(&msg, i));
                    break;
                }
            }
        }
        if p < buf.len() {
            t.push(Elt::Str(buf[p..].to_vec(), enc));
        }
        ans[i] = Some(t);
    }
    Ok(())
}

/// The first match of `re` in `s`: `Ok(None)` for none, `Err` the status
/// the loop stops at.
fn tre_first(re: &Regex, s: Subj<'_>) -> Result<Result<Option<Match>, tre::Status>, Fail> {
    let mut pm = [Match::UNSET];
    Ok(tre_exec(re, s, &mut pm, 0)?.map(|m| m.then_some(pm[0])))
}

/// ERE in wchar_t.
#[allow(clippy::too_many_arguments)]
fn split_wide(
    cx: &Cx,
    m: &Mode,
    this: Char<'_>,
    itok: usize,
    xs: &[Char<'_>],
    elts: impl Iterator<Item = usize>,
    ans: &mut [Pieces],
    rep: &mut Report,
) -> Result<(), Fail> {
    let Some(wsplit) = wtrans(this, m.loc, rep) else {
        return Err(Fail::Error(format!("'split' string {} is invalid", itok + 1)));
    };
    let re = Regex::new(&wsplit, tre::REG_EXTENDED, crate::text::ctype(m.loc))
        .map_err(|e| tre_fail(e, Some(&tr_char(this, m.loc)), rep))?;
    let ascii_split = this.is_ascii();
    for i in elts {
        let xi = xs[i];
        if xi == cx.na_string() {
            continue;
        }
        let Some(wbuf) = wide_input(xi, i, m.loc, rep) else { continue };
        let maybe_ascii = ascii_split && xi.is_ascii();
        let mut t = Vec::new();
        let mut p = 0;
        while p < wbuf.len() {
            let rest = &wbuf[p..];
            // (do_strsplit ignores tre_regwexec's errors)
            let Ok(Some(mt)) = tre_first(&re, Subj::Wide(rest))? else { break };
            let (so, eo) = (mt.so as usize, mt.eo as usize);
            if eo > 0 {
                // Match was non-empty.
                t.push(Elt::Str(wide_utf8(&rest[..so], so, maybe_ascii), Encoding::Utf8));
                p += eo;
            } else {
                // Match was empty: the next character, made with length
                // rm_so (0) as the C does
                t.push(Elt::Str(wide_utf8(&rest[..1], so, maybe_ascii), Encoding::Utf8));
                p += 1;
            }
        }
        if p < wbuf.len() {
            let rest = &wbuf[p..];
            t.push(Elt::Str(wide_utf8(rest, rest.len(), maybe_ascii), Encoding::Utf8));
        }
        ans[i] = Some(t);
    }
    Ok(())
}

/// ERE in normal chars -- single byte.
#[allow(clippy::too_many_arguments)]
fn split_bytes(
    cx: &Cx,
    m: &Mode,
    this: Char<'_>,
    itok: usize,
    xs: &[Char<'_>],
    elts: impl Iterator<Item = usize>,
    ans: &mut [Pieces],
    rep: &mut Report,
) -> Result<(), Fail> {
    // never use_UTF8
    let split = split_string(m, this, itok, rep)?;
    let compiled = if m.use_bytes {
        Regex::new_bytes(&split, tre::REG_EXTENDED, crate::text::ctype(m.loc))
    } else {
        tre_comp_native(&split, tre::REG_EXTENDED, m.loc)
    };
    let re = compiled.map_err(|e| tre_fail(e, Some(&String::from_utf8_lossy(&split)), rep))?;
    for i in elts {
        let xi = xs[i];
        if xi == cx.na_string() {
            continue;
        }
        let Some(buf) = input(xi, i, m.use_bytes, false, m.loc, rep) else { continue };
        let enc = if m.use_bytes { bytes_new(m.have_bytes_input) } else { mark_known(xi, m.loc) };
        let mut t = Vec::new();
        let mut p = 0;
        while p < buf.len() {
            let rest = &buf[p..];
            let mt = match tre_first(&re, Subj::Bytes(rest))? {
                Ok(Some(mt)) => mt,
                Ok(None) => break,
                Err(st) => {
                    // AFAICS the only possible error report is REG_ESPACE
                    if st == tre::Status::Espace {
                        rep.warn(format!("Out-of-memory error in regexp matching for element {}", i + 1));
                    }
                    break;
                }
            };
            let (so, eo) = (mt.so as usize, mt.eo as usize);
            if eo > 0 {
                // Match was non-empty.
                t.push(Elt::Str(rest[..so].to_vec(), enc));
                p += eo;
            } else {
                // Match was empty.
                t.push(Elt::Str(rest[..1].to_vec(), enc));
                p += 1;
            }
        }
        if p < buf.len() {
            t.push(Elt::Str(buf[p..].to_vec(), enc));
        }
        ans[i] = Some(t);
    }
    Ok(())
}
