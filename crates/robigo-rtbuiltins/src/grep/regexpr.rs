//! `.Internal(regexpr(pattern, text, ignore.case, perl, fixed, useBytes))`,
//! `gregexpr` (the same arguments) and `.Internal(regexec(pattern, text,
//! ignore.case, fixed, useBytes))`.

use super::{
    Fail, Pcre, Report, Subj, Text, fgrep_one, have, input, only_ascii, pattern, pcre_exec_error, strings,
    tre_comp_native, tre_exec, tre_fail, utf8clen, wide_input, wtrans,
};
use crate::attrib::{Attr, Attrs};
use crate::util::{as_bool2, as_logical};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Built, Char, Cx, Encoding, Lgl, Locale, NeedGc, Out, RResult, Ref, Root};
use robigo_text::tre::{self, Match, Regex};

const NA_INTEGER: i32 = i32::MIN;

/// How the pattern matches.
enum Matcher {
    Fixed(Vec<u8>),
    Perl(Pcre),
    Tre(Regex),
}

/// The matches of one string (`gregexpr`) or of all of them (`regexpr`):
/// starts from one and lengths, and the groups' (`perl = TRUE`) by column.
#[derive(Default)]
struct Hits {
    st: Vec<i32>,
    ml: Vec<i32>,
    cs: Vec<i32>,
    cl: Vec<i32>,
}

/// One element of `gregexpr`'s result.
enum GElt {
    /// `gregexpr_NAInputAns()`.
    Na,
    /// `gregexpr_BadStringAns()`.
    Bad,
    Hits(Hits),
}

/// What phase 2 found.
enum Found {
    Regexpr(Hits),
    Gregexpr(Vec<GElt>),
}

/// Phase 2's result: the matches, whether the attributes say "useBytes",
/// `itype`, and PCRE's group names.
struct Res {
    found: Found,
    use_bytes: bool,
    itype_bytes: bool,
    names: Option<Vec<String>>,
}

/// `regexpr` (`CODE` 0) and `gregexpr` (1).
// R: grep.c:do_regexpr
pub fn do_regexpr<const CODE: u8>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let global = CODE == 1;
    let row = if global { crate::row!("gregexpr", true) } else { crate::row!("regexpr", true) };
    row.check_arity(cx, call, args)?;
    let pat = args.root(0);
    let text = args.root(1);
    let igcase_opt = as_bool2(cx, call, args.root(2))?;
    let mut perl_opt = as_bool2(cx, call, args.root(3))?;
    let fixed_opt = as_bool2(cx, call, args.root(4))?;
    let use_bytes = as_bool2(cx, call, args.root(5))?;
    if fixed_opt && igcase_opt {
        cx.warning("argument 'ignore.case = TRUE' will be ignored")?;
    }
    if fixed_opt && perl_opt {
        cx.warning("argument 'perl = TRUE' will be ignored")?;
        perl_opt = false;
    }
    // Note that excluding NAs differs from grep/sub
    let pat_ok = {
        let p = pat.bind(cx);
        p.sexptype() == STRSXP && !strings(p).is_empty() && strings(p).get(0) != cx.na_string()
    };
    if !pat_ok {
        return Err(cx.error("invalid 'pattern' argument"));
    }
    if strings(pat.bind(cx)).len() > 1 {
        cx.warning("argument 'pattern' has length > 1 and only the first element will be used")?;
    }
    if text.bind(cx).sexptype() != STRSXP {
        return Err(cx.error("invalid 'text' argument"));
    }
    let loc = cx.locale();
    let mut rep = Report::default();
    let r = {
        let cx: &Cx = cx;
        let tv = strings(text.bind(cx));
        let xs: Vec<Char<'_>> = (0..tv.len()).map(|i| tv.get(i)).collect();
        let p0 = strings(pat.bind(cx)).get(0);
        let o = Opts { igcase: igcase_opt, perl: perl_opt, fixed: fixed_opt, use_bytes, loc, global };
        regexpr(cx, p0, &xs, o, &mut rep)
    };
    let res = rep.finish(cx, call, r)?;
    cx.build_with(out, |cx| {
        let common = Common::new(cx, &res)?;
        match &res.found {
            Found::Regexpr(h) => Ok(Built::done(common.hits(cx, h, h.st.len())?)),
            Found::Gregexpr(elts) => {
                let mut ans = cx.alloc_vector::<Ref<'_>>(elts.len())?;
                for (i, e) in elts.iter().enumerate() {
                    let v = match e {
                        GElt::Na => bare(cx, NA_INTEGER)?,
                        GElt::Bad => bare(cx, -1)?,
                        GElt::Hits(h) => common.hits(cx, h, h.st.len())?,
                    };
                    ans.set(i, v);
                }
                Ok(Built::done(ans.finish()))
            }
        }
    })
}

/// The attribute values every element shares.
struct Common<'g> {
    use_bytes: bool,
    itype: Ref<'g>,
    /// `capture_names` and the `dimnames` `list(NULL, capture_names)`.
    names: Option<(Ref<'g>, Ref<'g>)>,
}

impl<'g> Common<'g> {
    fn new(cx: &'g Cx, res: &Res) -> Result<Common<'g>, NeedGc> {
        let mut itype = cx.alloc_vector::<Char<'_>>(1)?;
        let it: &[u8] = if res.itype_bytes { b"bytes" } else { b"chars" };
        itype.set(0, cx.mk_char(it, Encoding::Native)?);
        let names = match &res.names {
            None => None,
            Some(nm) => {
                let mut cn = cx.alloc_vector::<Char<'_>>(nm.len())?;
                for (i, s) in nm.iter().enumerate() {
                    cn.set(i, cx.mk_char(s.as_bytes(), Encoding::Native)?);
                }
                let cn: Ref<'_> = cn.finish().into();
                let mut dmn = cx.alloc_vector::<Ref<'_>>(2)?;
                dmn.set(0, cx.nil());
                dmn.set(1, cn);
                Some((cn, dmn.finish().into()))
            }
        };
        Ok(Common { use_bytes: res.use_bytes, itype: itype.finish().into(), names })
    }

    /// The integer vector of `h`'s starts with its attributes: match.length,
    /// index.type and useBytes with `useBytes`, and the capture matrices
    /// (`rows` by the number of groups) and names with PCRE's groups.
    fn hits(&self, cx: &'g Cx, h: &Hits, rows: usize) -> Result<Ref<'g>, NeedGc> {
        let mut ans = cx.alloc_copy(&h.st)?;
        let mut a = Attrs::new();
        let ml = cx.alloc_copy(&h.ml)?.finish();
        a.install(Attr { tag: cx.install(b"match.length")?, value: ml.into() });
        if self.use_bytes {
            a.install(Attr { tag: cx.install(b"index.type")?, value: self.itype });
            let mut t = cx.alloc_vector::<Lgl>(1)?;
            t.as_mut_slice()[0] = Lgl::TRUE;
            a.install(Attr { tag: cx.install(b"useBytes")?, value: t.finish().into() });
        }
        if let Some((cn, dmn)) = self.names {
            let cc = cn.len().unwrap_or(0);
            let matrix = |v: &[i32]| -> Result<Ref<'g>, NeedGc> {
                let mut m = cx.alloc_copy(v)?;
                let mut ma = Attrs::new();
                let dim = cx.alloc_copy(&[rows as i32, cc as i32])?.finish();
                ma.install(Attr { tag: cx.install(b"dim")?, value: dim.into() });
                ma.install(Attr { tag: cx.install(b"dimnames")?, value: dmn });
                ma.apply(cx, &mut m)?;
                Ok(m.finish().into())
            };
            a.install(Attr { tag: cx.install(b"capture.start")?, value: matrix(&h.cs)? });
            a.install(Attr { tag: cx.install(b"capture.length")?, value: matrix(&h.cl)? });
            a.install(Attr { tag: cx.install(b"capture.names")?, value: cn });
        }
        a.apply(cx, &mut ans)?;
        Ok(ans.finish().into())
    }
}

/// `gregexpr_NAInputAns()` (`v` NA) and `gregexpr_BadStringAns()` (-1).
// R: grep.c:gregexpr_NAInputAns
fn bare(cx: &Cx, v: i32) -> Result<Ref<'_>, NeedGc> {
    let mut ans = cx.alloc_copy(&[v])?;
    let ml = cx.alloc_copy(&[v])?.finish();
    let mut a = Attrs::new();
    a.install(Attr { tag: cx.install(b"match.length")?, value: ml.into() });
    a.apply(cx, &mut ans)?;
    Ok(ans.finish().into())
}

#[derive(Clone, Copy)]
struct Opts {
    igcase: bool,
    perl: bool,
    fixed: bool,
    use_bytes: bool,
    loc: Locale,
    global: bool,
}

/// The number of characters of a (valid) UTF-8 string.
fn utf8_chars(s: &[u8]) -> usize {
    s.iter().filter(|&&b| b & 0xc0 != 0x80).count()
}

/// Phase 2.
fn regexpr(cx: &Cx, p0: Char<'_>, xs: &[Char<'_>], o: Opts, rep: &mut Report) -> Result<Res, Fail> {
    let Opts { igcase, perl, fixed, loc, global, .. } = o;
    let n = xs.len();
    let pats = [p0];
    let mut use_bytes = o.use_bytes;
    if !use_bytes {
        use_bytes = have(&pats, Encoding::Bytes) || have(xs, Encoding::Bytes);
    }
    let itype_bytes = use_bytes;
    if !use_bytes {
        use_bytes = only_ascii(cx, &pats) && only_ascii(cx, xs);
    }
    let mut use_utf8 = false;
    if !use_bytes {
        // As from R 2.10.0 we use UTF-8 mode in PCRE in all MBCS locales,
        // and as from 2.11.0 in TRE too.
        if !fixed && loc.utf8 {
            use_utf8 = true;
        }
        if !use_utf8 {
            use_utf8 = have(&pats, Encoding::Utf8) || have(xs, Encoding::Utf8);
        }
        // (not a latin1 locale)
        if !use_utf8 {
            use_utf8 = have(&pats, Encoding::Latin1) || have(xs, Encoding::Latin1);
        }
    }
    let mut use_wc = false;
    if !fixed && !perl {
        use_wc = use_utf8;
        use_utf8 = false;
    }
    let mut spat: Vec<u8> = Vec::new();
    let m = if use_wc {
        let Some(wpat) = wtrans(p0, loc, rep) else {
            return Err(Fail::Error("regular expression is invalid".into()));
        };
        let cflags = tre::REG_EXTENDED | if igcase { tre::REG_ICASE } else { 0 };
        Matcher::Tre(Regex::new(&wpat, cflags, crate::text::ctype(loc)).map_err(|e| tre_fail(e, None, rep))?)
    } else {
        spat = pattern(p0, || "regular expression".to_string(), use_bytes, use_utf8, loc, rep)?.into_owned();
        if fixed {
            Matcher::Fixed(spat.clone())
        } else if perl {
            Matcher::Perl(Pcre::prepare(&spat, use_utf8, igcase, loc, rep)?)
        } else {
            let cflags = tre::REG_EXTENDED | if igcase { tre::REG_ICASE } else { 0 };
            Matcher::Tre(
                Regex::new_bytes(&spat, cflags, crate::text::ctype(loc))
                    .map_err(|e| tre_fail(e, Some(&String::from_utf8_lossy(&spat)), rep))?,
            )
        }
    };
    let names = match &m {
        // (`if (perl_opt && capture_count)`: no groups, no capture attributes)
        Matcher::Perl(re) => Some(re.capture_names()).filter(|v| !v.is_empty()),
        _ => None,
    };
    let cc = names.as_ref().map_or(0, |v| v.len());
    // the pattern's length in characters, for `fixed = TRUE`
    let patlen = if !use_bytes && (use_utf8 || loc.utf8) { utf8_chars(&spat) } else { spat.len() } as i32;
    let found = if !global {
        if cc > 0 && n > i32::MAX as usize {
            return Err(Fail::Error("too long a vector".into()));
        }
        let mut h = Hits { st: vec![0; n], ml: vec![0; n], cs: vec![NA_INTEGER; n * cc], cl: vec![NA_INTEGER; n * cc] };
        for (i, &xi) in xs.iter().enumerate() {
            if xi == cx.na_string() {
                h.st[i] = NA_INTEGER;
                h.ml[i] = NA_INTEGER;
                continue;
            }
            let s = if use_wc {
                match wide_input(xi, i, loc, rep) {
                    Some(w) => Text::Wide(w),
                    None => {
                        (h.st[i], h.ml[i]) = (-1, -1);
                        continue;
                    }
                }
            } else {
                match input(xi, i, use_bytes, use_utf8, loc, rep) {
                    Some(b) => Text::Bytes(b),
                    None => {
                        (h.st[i], h.ml[i]) = (-1, -1);
                        continue;
                    }
                }
            };
            match (&m, s.subj()) {
                (Matcher::Fixed(p), Subj::Bytes(b)) => {
                    let st = fgrep_one(p, b, use_bytes, use_utf8, loc).map(|(st, _)| st);
                    (h.st[i], h.ml[i]) = match st {
                        Some(st) => (st as i32 + 1, patlen),
                        None => (-1, -1),
                    };
                }
                (Matcher::Perl(re), Subj::Bytes(b)) => {
                    let mut locs = re.locations();
                    match re.match_at(&mut locs, b, 0, false)? {
                        Ok(true) => {
                            let (st, ml) = extract(use_utf8, b, locs.get(0));
                            (h.st[i], h.ml[i]) = (st, ml);
                            for g in 0..cc {
                                (h.cs[i + g * n], h.cl[i + g * n]) = extract(use_utf8, b, locs.get(g + 1));
                            }
                        }
                        r => {
                            if let Err(msg) = r {
                                rep.warn(pcre_exec_error(&msg, i));
                            }
                            (h.st[i], h.ml[i]) = (-1, -1);
                            for g in 0..cc {
                                (h.cs[i + g * n], h.cl[i + g * n]) = (-1, -1);
                            }
                        }
                    }
                }
                (Matcher::Tre(re), s) => {
                    let mut pm = [Match::UNSET];
                    let rc = tre_exec(re, s, &mut pm, 0)?;
                    (h.st[i], h.ml[i]) = match rc {
                        Ok(true) => (pm[0].so + 1, pm[0].eo - pm[0].so),
                        _ => (-1, -1),
                    };
                    // AFAICS the only possible error report is REG_ESPACE
                    if rc == Err(tre::Status::Espace) {
                        rep.warn(format!("Out-of-memory error in regexp matching for element {}", i + 1));
                    }
                }
                _ => unreachable!("robigo: fixed and PCRE matching are on bytes"),
            }
        }
        Found::Regexpr(h)
    } else {
        let mut elts = Vec::with_capacity(n);
        for (i, &xi) in xs.iter().enumerate() {
            if xi == cx.na_string() {
                elts.push(GElt::Na);
                continue;
            }
            let e = match &m {
                Matcher::Fixed(_) | Matcher::Perl(_) => match input(xi, i, use_bytes, use_utf8, loc, rep) {
                    None => GElt::Bad,
                    Some(s) => match &m {
                        Matcher::Fixed(p) => GElt::Hits(gregexpr_fixed(p, &s, use_bytes, use_utf8, patlen, loc)),
                        Matcher::Perl(re) => GElt::Hits(pcre_gregexpr(re, &s, use_utf8, cc, i, rep)?),
                        Matcher::Tre(_) => unreachable!(),
                    },
                },
                Matcher::Tre(re) => gregexpr_regexc(re, xi, use_bytes, use_wc, i, loc, rep)?,
            };
            elts.push(e);
        }
        Found::Gregexpr(elts)
    };
    Ok(Res { found, use_bytes, itype_bytes, names })
}

/// `getNc(s, st)`: the characters in the first `st` bytes of UTF-8 `s`.
// R: grep.c:getNc
fn get_nc(s: &[u8], st: usize) -> i32 {
    let (mut i, mut nc) = (0, 0);
    while i < st {
        i += utf8clen(s[i]);
        nc += 1;
    }
    nc
}

/// `ovector_extract_start_length`: a group's start from one and length,
/// in characters with `use_UTF8`. An unset group (`PCRE2_UNSET`, -1 as an
/// `int`) is 0 and 0.
// R: grep.c:ovector_extract_start_length
fn extract(use_utf8: bool, s: &[u8], g: Option<(usize, usize)>) -> (i32, i32) {
    let Some((so, eo)) = g else { return (0, 0) };
    let st = so as i32;
    let mut m = st + 1;
    let mut len = eo as i32 - st;
    if use_utf8 {
        // Unfortunately these are in bytes
        if st > 0 {
            m = 1 + get_nc(s, so);
        }
        len = get_nc(&s[so..], len as usize);
    }
    (m, len)
}

/// `gregexpr_fixed(pattern, string, useBytes, use_UTF8, itype)`.
// R: grep.c:gregexpr_fixed
fn gregexpr_fixed(pat: &[u8], s: &[u8], use_bytes: bool, use_utf8: bool, patlen: i32, loc: Locale) -> Hits {
    let slen = s.len();
    let mut h = Hits::default();
    let Some((mut st, mut nb)) = fgrep_one(pat, s, use_bytes, use_utf8, loc) else {
        h.st.push(-1);
        h.ml.push(-1);
        return h;
    };
    h.st.push(st as i32 + 1);
    h.ml.push(patlen);
    let (mut string, mut curpos) = (0usize, 0usize);
    loop {
        string += nb;
        if patlen == 0 {
            curpos += st + 1;
        } else {
            curpos += st + patlen as usize;
        }
        if curpos >= slen {
            break;
        }
        match fgrep_one(pat, &s[string..], use_bytes, use_utf8, loc) {
            Some((st1, nb1)) => {
                (st, nb) = (st1, nb1);
                h.st.push((curpos + st + 1) as i32);
                h.ml.push(patlen);
            }
            None => break,
        }
    }
    h
}

/// `R_pcre2_gregexpr(...)`: every match from the subject's start, with no
/// `PCRE2_NOTBOL`.
// R: grep.c:R_pcre2_gregexpr
fn pcre_gregexpr(re: &Pcre, s: &[u8], use_utf8: bool, cc: usize, i: usize, rep: &mut Report) -> Result<Hits, Fail> {
    let slen = s.len();
    let mut start = 0;
    let mut locs = re.locations();
    let (mut st, mut ml) = (Vec::new(), Vec::new());
    let mut caps: Vec<(Vec<i32>, Vec<i32>)> = vec![(Vec::new(), Vec::new()); cc];
    loop {
        if use_utf8 && start < slen && s[start] & 0xc0 == 0x80 {
            // GNU R matches from inside a character with PCRE2_NO_UTF_CHECK,
            // which the `pcre2` crate does not pass
            return Err(Fail::NotYet(
                "a PCRE match from inside a UTF-8 character (gregexpr after an empty match)".into(),
            ));
        }
        match re.match_at(&mut locs, s, start, false)? {
            Ok(true) => {
                let (m, l) = extract(use_utf8, s, locs.get(0));
                st.push(m);
                ml.push(l);
                for (g, (cs, cl)) in caps.iter_mut().enumerate() {
                    let (a, b) = extract(use_utf8, s, locs.get(g + 1));
                    cs.push(a);
                    cl.push(b);
                }
                let (so, eo) = locs.get(0).expect("robigo: a match has offsets");
                // we need to advance 'start' in bytes
                start = if eo == so { so + 1 } else { eo };
                if start >= slen {
                    break;
                }
            }
            r => {
                if let Err(msg) = r {
                    rep.warn(pcre_exec_error(&msg, i));
                }
                break;
            }
        }
    }
    if st.is_empty() {
        st.push(-1);
        ml.push(-1);
        for (cs, cl) in caps.iter_mut() {
            cs.push(-1);
            cl.push(-1);
        }
    }
    let cs = caps.iter().flat_map(|c| c.0.iter().copied()).collect();
    let cl = caps.iter().flat_map(|c| c.1.iter().copied()).collect();
    Ok(Hits { st, ml, cs, cl })
}

/// `gregexpr_Regexc(reg, sstr, useBytes, use_WC, i, itype, &nwarn)`.
// R: grep.c:gregexpr_Regexc
fn gregexpr_regexc(
    re: &Regex,
    c: Char<'_>,
    use_bytes: bool,
    use_wc: bool,
    i: usize,
    loc: Locale,
    rep: &mut Report,
) -> Result<GElt, Fail> {
    let s = if use_wc {
        match wide_input(c, i, loc, rep) {
            Some(w) => Text::Wide(w),
            None => return Ok(GElt::Bad),
        }
    } else {
        match input(c, i, use_bytes, false, loc, rep) {
            Some(b) => Text::Bytes(b),
            None => return Ok(GElt::Bad),
        }
    };
    let len = match &s {
        Text::Bytes(b) => b.len(),
        Text::Wide(w) => w.len(),
    };
    let mut h = Hits::default();
    let (mut offset, mut eflags) = (0usize, 0);
    loop {
        // in case offset>=len (e.g., when len==0)
        let mut rc = Ok(false);
        let mut pm = [Match::UNSET];
        if offset < len {
            let sub = match &s {
                Text::Bytes(b) => Subj::Bytes(&b[offset..]),
                Text::Wide(w) => Subj::Wide(&w[offset..]),
            };
            rc = tre_exec(re, sub, &mut pm, eflags)?;
        }
        let found = rc == Ok(true);
        if found {
            let st = pm[0].so;
            h.st.push((offset as i32) + st + 1); // index from one
            let l = pm[0].eo - st;
            h.ml.push(l);
            if l == 0 {
                offset += st as usize + 1;
            } else {
                offset += pm[0].eo as usize;
            }
        } else if h.st.is_empty() {
            h.st.push(-1);
            h.ml.push(-1);
        }
        eflags = tre::REG_NOTBOL;
        // AFAICS the only possible error report is REG_ESPACE
        if rc == Err(tre::Status::Espace) {
            rep.warn(format!("Out-of-memory error in regexp matching for element {}", i + 1));
        }
        if !found {
            break;
        }
    }
    Ok(GElt::Hits(h))
}

/// `.Internal(regexec(pattern, text, ignore.case, fixed, useBytes))`: TRE's
/// match and groups of each string (`perl = TRUE` is R code over
/// `regexpr`).
// R: grep.c:do_regexec
pub fn do_regexec(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("regexec", true).check_arity(cx, call, args)?;
    let pat = args.root(0);
    let text = args.root(1);
    let na0 = |v: i32| if v == NA_INTEGER { 0 } else { v };
    let mut opt_icase = na0(as_logical(cx, args.get(cx, 2)));
    let opt_fixed = na0(as_logical(cx, args.get(cx, 3)));
    let use_bytes = na0(as_logical(cx, args.get(cx, 4))) != 0;
    if opt_fixed != 0 && opt_icase != 0 {
        cx.warning("argument 'ignore.case = TRUE' will be ignored")?;
        opt_icase = 0;
    }
    let mut cflags = tre::REG_EXTENDED;
    if opt_fixed != 0 {
        cflags |= tre::REG_LITERAL;
    }
    if opt_icase != 0 {
        cflags |= tre::REG_ICASE;
    }
    let pat_ok = {
        let p = pat.bind(cx);
        p.sexptype() == STRSXP && !strings(p).is_empty() && strings(p).get(0) != cx.na_string()
    };
    if !pat_ok {
        return Err(cx.error("invalid 'pattern' argument"));
    }
    if strings(pat.bind(cx)).len() > 1 {
        cx.warning("argument 'pattern' has length > 1 and only the first element will be used")?;
    }
    if text.bind(cx).sexptype() != STRSXP {
        return Err(cx.error("invalid 'text' argument"));
    }
    let loc = cx.locale();
    let mut rep = Report::default();
    let r = {
        let cx: &Cx = cx;
        let tv = strings(text.bind(cx));
        let xs: Vec<Char<'_>> = (0..tv.len()).map(|i| tv.get(i)).collect();
        let p0 = strings(pat.bind(cx)).get(0);
        regexec(cx, p0, &xs, cflags, opt_fixed != 0, use_bytes, loc, &mut rep)
    };
    let (elts, use_bytes, itype_bytes) = rep.finish(cx, call, r)?;
    cx.build_with(out, |cx| {
        let res = Res { found: Found::Gregexpr(Vec::new()), use_bytes, itype_bytes, names: None };
        let common = Common::new(cx, &res)?;
        let mut ans = cx.alloc_vector::<Ref<'_>>(elts.len())?;
        for (i, e) in elts.iter().enumerate() {
            let v = match e {
                // ScalarInteger(NA) with match.length NA
                GElt::Na => bare(cx, NA_INTEGER)?,
                GElt::Bad => unreachable!("robigo: regexec errors on an invalid string"),
                GElt::Hits(h) => common.hits(cx, h, h.st.len())?,
            };
            ans.set(i, v);
        }
        Ok(Built::done(ans.finish()))
    })
}

/// Phase 2 of `regexec`: each string's match and groups, whether the
/// attributes say "useBytes", and `itype`.
#[allow(clippy::too_many_arguments)]
fn regexec(
    cx: &Cx,
    p0: Char<'_>,
    xs: &[Char<'_>],
    cflags: i32,
    fixed: bool,
    use_bytes: bool,
    loc: Locale,
    rep: &mut Report,
) -> Result<(Vec<GElt>, bool, bool), Fail> {
    let pats = [p0];
    let mut use_bytes = use_bytes;
    if !use_bytes {
        use_bytes = have(&pats, Encoding::Bytes) || have(xs, Encoding::Bytes);
    }
    let itype_bytes = use_bytes;
    if !use_bytes {
        use_bytes = only_ascii(cx, &pats) && only_ascii(cx, xs);
    }
    let mut use_wc = false;
    if !use_bytes {
        if !fixed && loc.utf8 {
            use_wc = true;
        }
        if !use_wc {
            use_wc = have(&pats, Encoding::Utf8) || have(xs, Encoding::Utf8);
        }
        // (not a latin1 locale)
        if !use_wc {
            use_wc = have(&pats, Encoding::Latin1) || have(xs, Encoding::Latin1);
        }
    }
    let regcomp_error = |e: tre::Error| match e {
        tre::Error::Status(s) => Fail::Error(format!("regcomp error: '{}'", s.message())),
        e => super::exec_fail(e),
    };
    let ctype = crate::text::ctype(loc);
    let re = if use_bytes {
        Regex::new_bytes(p0.bytes(), cflags, ctype).map_err(regcomp_error)?
    } else if use_wc {
        let Some(ws) = wtrans(p0, loc, rep) else {
            return Err(Fail::Error("regular expression is invalid".into()));
        };
        Regex::new(&ws, cflags, ctype).map_err(regcomp_error)?
    } else {
        let s = pattern(p0, || "regular expression".to_string(), false, false, loc, rep)?;
        tre_comp_native(&s, cflags, loc).map_err(regcomp_error)?
    };
    let nmatch = re.re_nsub() + 1;
    let mut pm = vec![Match::UNSET; nmatch];
    let mut elts = Vec::with_capacity(xs.len());
    for (i, &xi) in xs.iter().enumerate() {
        if xi == cx.na_string() {
            elts.push(GElt::Na);
            continue;
        }
        let invalid = || Fail::Error(format!("input string {} is invalid in this locale", i + 1));
        let rc = if use_bytes {
            tre_exec(&re, Subj::Bytes(xi.bytes()), &mut pm, 0)?
        } else if use_wc {
            let Some(wt) = wtrans(xi, loc, rep) else { return Err(invalid()) };
            tre_exec(&re, Subj::Wide(&wt), &mut pm, 0)?
        } else {
            let t = super::tr_native(xi, loc, rep).filter(|b| !loc.utf8 || super::mbcs_valid(b));
            let Some(t) = t else { return Err(invalid()) };
            // tre_regexec: multibyte in a UTF-8 locale, bytes in a single-byte one
            let r = if loc.utf8 { re.exec_mbs(&t, &mut pm, 0) } else { re.exec_bytes(&t, &mut pm, 0) };
            match r {
                Ok(m) => Ok(m),
                Err(tre::Error::Status(st)) => Err(st),
                Err(e) => return Err(super::exec_fail(e)),
            }
        };
        let h = if rc == Ok(true) {
            Hits {
                st: pm.iter().map(|m| m.so + 1).collect(),
                ml: pm.iter().map(|m| m.eo - m.so).collect(),
                ..Hits::default()
            }
        } else {
            // No match (or could there be an error?).
            // AFAICS the only possible error report is REG_ESPACE
            if rc == Err(tre::Status::Espace) {
                rep.warn(format!("Out-of-memory error in regexp matching for element {}", i + 1));
            }
            Hits { st: vec![-1], ml: vec![-1], ..Hits::default() }
        };
        elts.push(GElt::Hits(h));
    }
    Ok((elts, use_bytes, itype_bytes))
}
