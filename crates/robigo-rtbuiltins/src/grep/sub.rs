//! `.Internal(sub(pattern, replacement, x, ignore.case, perl, fixed,
//! useBytes))` and `gsub`.

use super::{
    Elt, Fail, Pcre, Report, Subj, bytes_new, fgrep_one_bytes, have, mark_bytes_old, mark_known, mbcs_valid,
    only_ascii, pattern, pcre_exec_error, strings, tr_char, tr_native, tr_utf8, tre_exec, tre_fail, utf8_valid,
    utf8clen, wide_utf8, wtrans,
};
use crate::attrib::Attrs;
use crate::text::ctype;
use crate::util::as_bool2;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Built, Char, Cx, Encoding, Locale, Out, RResult, Root};
use robigo_text::tre::{self, Match, Regex};
use std::borrow::Cow;

/// `sub` (`CODE` 0) and `gsub` (1).
// R: grep.c:do_gsub
pub fn do_gsub<const CODE: u8>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let global = CODE == 1;
    let row = if global { crate::row!("gsub", true) } else { crate::row!("sub", true) };
    row.check_arity(cx, call, args)?;
    let pat = args.root(0);
    let rep = args.root(1);
    let text = args.root(2);
    let igcase_opt = as_bool2(cx, call, args.root(3))?;
    let mut perl_opt = as_bool2(cx, call, args.root(4))?;
    let fixed_opt = as_bool2(cx, call, args.root(5))?;
    let use_bytes = as_bool2(cx, call, args.root(6))?;
    if fixed_opt && igcase_opt {
        cx.warning("argument 'ignore.case = TRUE' will be ignored")?;
    }
    if fixed_opt && perl_opt {
        cx.warning("argument 'perl = TRUE' will be ignored")?;
        perl_opt = false;
    }
    let str_len = |cx: &Cx, x: Root<'_>| {
        let x = x.bind(cx);
        (x.sexptype() == STRSXP).then(|| strings(x).len())
    };
    let pn = str_len(cx, pat).unwrap_or(0);
    if pn < 1 {
        return Err(cx.error("invalid 'pattern' argument"));
    }
    if pn > 1 {
        cx.warning("argument 'pattern' has length > 1 and only the first element will be used")?;
    }
    let rn = str_len(cx, rep).unwrap_or(0);
    if rn < 1 {
        return Err(cx.error("invalid 'replacement' argument"));
    }
    if rn > 1 {
        cx.warning("argument 'replacement' has length > 1 and only the first element will be used")?;
    }
    let Some(n) = str_len(cx, text) else {
        return Err(cx.error("invalid 'text' argument"));
    };
    let loc = cx.locale();
    // This contradicts the code below that has NA matching NA
    let elts: Option<Vec<Elt>> = if strings(pat.bind(cx)).get(0) == cx.na_string() {
        None
    } else {
        let mut report = Report::default();
        let r = {
            let cx: &Cx = cx;
            let p0 = strings(pat.bind(cx)).get(0);
            let r0 = strings(rep.bind(cx)).get(0);
            let tv = strings(text.bind(cx));
            let xs: Vec<Char<'_>> = (0..n).map(|i| tv.get(i)).collect();
            let o = Opts { igcase: igcase_opt, perl: perl_opt, fixed: fixed_opt, use_bytes, global, loc };
            substitute(cx, p0, r0, &xs, o, &mut report)
        };
        Some(report.finish(cx, call, r)?)
    };
    cx.build_with(out, |cx| {
        let mut ans = cx.alloc_vector::<Char<'_>>(n)?;
        match &elts {
            None => (0..n).for_each(|i| ans.set(i, cx.na_string())),
            Some(es) => {
                for (i, e) in es.iter().enumerate() {
                    ans.set(i, e.make(cx)?);
                }
            }
        }
        // SHALLOW_DUPLICATE_ATTRIB(ans, text): this also copies the class,
        // if any
        let mut a = Attrs::new();
        a.copy_all(text.bind(cx));
        a.apply(cx, &mut ans)?;
        Ok(Built::done(ans.finish()))
    })
}

#[derive(Clone, Copy)]
struct Opts {
    igcase: bool,
    perl: bool,
    fixed: bool,
    use_bytes: bool,
    global: bool,
    loc: Locale,
}

/// How the pattern matches, and the replacement in the same units.
enum Matcher {
    Fixed(Vec<u8>, Vec<u8>),
    Perl(Pcre, Vec<u8>),
    TreBytes(Regex, Vec<u8>),
    TreWide(Regex, Vec<i32>, bool),
}

/// Phase 2: every string with its matches replaced.
fn substitute(
    cx: &Cx,
    p0: Char<'_>,
    r0: Char<'_>,
    xs: &[Char<'_>],
    o: Opts,
    rep: &mut Report,
) -> Result<Vec<Elt>, Fail> {
    let Opts { igcase, perl, fixed, global, loc, .. } = o;
    let (pats, reps) = ([p0], [r0]);
    let have_bytes_input = have(&pats, Encoding::Bytes) || have(&reps, Encoding::Bytes) || have(xs, Encoding::Bytes);
    let mut use_bytes = o.use_bytes;
    if !use_bytes {
        use_bytes = only_ascii(cx, &pats) && only_ascii(cx, &reps) && only_ascii(cx, xs);
    }
    if !use_bytes {
        use_bytes = have_bytes_input;
    }
    let mut use_utf8 = false;
    if !use_bytes {
        // if we have non-ASCII text in a DBCS locale, we need to use wchar
        // in TRE
        if !fixed && loc.utf8 {
            use_utf8 = true;
        }
        if !use_utf8 {
            use_utf8 = have(&pats, Encoding::Utf8) || have(&reps, Encoding::Utf8) || have(xs, Encoding::Utf8);
        }
        // (not a latin1 locale)
        if !use_utf8 {
            use_utf8 = have(&pats, Encoding::Latin1) || have(&reps, Encoding::Latin1) || have(xs, Encoding::Latin1);
        }
    }
    let mut use_wc = false;
    if !fixed && !perl {
        use_wc = use_utf8;
        use_utf8 = false;
    }
    let na = cx.na_string();
    let rep_na = r0 == na;
    let cflags = tre::REG_EXTENDED | if igcase { tre::REG_ICASE } else { 0 };
    let m = if use_wc {
        let Some(wpat) = wtrans(p0, loc, rep) else {
            return Err(Fail::Error("'pattern' is invalid".into()));
        };
        let wrep = if rep_na {
            vec![b'N' as i32, b'A' as i32]
        } else {
            wtrans(r0, loc, rep).ok_or_else(|| Fail::Error("'replacement' is invalid".into()))?
        };
        // (NA_STRING is not marked ASCII)
        let ascii_patrep = p0.is_ascii() && !rep_na && r0.is_ascii();
        let re = Regex::new(&wpat, cflags, ctype(loc)).map_err(|e| tre_fail(e, Some(&tr_char(p0, loc)), rep))?;
        Matcher::TreWide(re, wrep, ascii_patrep)
    } else {
        let spat = pattern(p0, || "'pattern'".into(), use_bytes, use_utf8, loc, rep)?;
        let srep: Cow<'_, [u8]> = if rep_na {
            Cow::Borrowed(b"NA")
        } else if use_bytes {
            Cow::Borrowed(r0.bytes())
        } else if use_utf8 {
            tr_utf8(r0, loc, rep)
                .filter(|b| utf8_valid(b))
                .ok_or_else(|| Fail::Error("'replacement' is invalid UTF-8".into()))?
        } else {
            tr_native(r0, loc, rep)
                .filter(|b| !loc.utf8 || mbcs_valid(b))
                .ok_or_else(|| Fail::Error("'replacement' is invalid in this locale".into()))?
        };
        if fixed {
            if spat.is_empty() {
                return Err(Fail::Error("zero-length pattern".into()));
            }
            Matcher::Fixed(spat.into_owned(), srep.into_owned())
        } else if perl {
            Matcher::Perl(Pcre::prepare(&spat, use_utf8, igcase, loc, rep)?, srep.into_owned())
        } else {
            let re = Regex::new_bytes(&spat, cflags, ctype(loc))
                .map_err(|e| tre_fail(e, Some(&String::from_utf8_lossy(&spat)), rep))?;
            Matcher::TreBytes(re, srep.into_owned())
        }
    };
    // the mark of a string made from the input
    let new_mark = |xi: Char<'_>| {
        if use_bytes {
            bytes_new(have_bytes_input)
        } else if use_utf8 {
            Encoding::Utf8
        } else {
            mark_known(xi, loc)
        }
    };
    let mut ans = Vec::with_capacity(xs.len());
    for (i, &xi) in xs.iter().enumerate() {
        // NA pattern was handled above
        if xi == na {
            ans.push(Elt::Na);
            continue;
        }
        let made: Option<Elt> = if let Matcher::TreWide(re, wrep, ascii_patrep) = &m {
            let Some(ws) = wtrans(xi, loc, rep) else {
                return Err(Fail::Error(format!("input string {} is invalid", i + 1)));
            };
            let ascii_texti = xi.is_ascii();
            tre_sub(re, &ws, wrep, global, i, false, rep)?.map(|(mut buf, offset)| {
                buf.extend_from_slice(&ws[offset..]);
                let len = buf.len();
                Elt::Str(wide_utf8(&buf, len, *ascii_patrep && ascii_texti), Encoding::Utf8)
            })
        } else {
            let s: Cow<'_, [u8]> = if use_bytes {
                Cow::Borrowed(xi.bytes())
            } else if use_utf8 {
                tr_utf8(xi, loc, rep)
                    .filter(|b| utf8_valid(b))
                    .ok_or_else(|| Fail::Error(format!("input string {} is invalid UTF-8", i + 1)))?
            } else {
                tr_native(xi, loc, rep)
                    .filter(|b| !loc.utf8 || mbcs_valid(b))
                    .ok_or_else(|| Fail::Error(format!("input string {} is invalid in this locale", i + 1)))?
            };
            let done = match &m {
                Matcher::Fixed(spat, srep) => fixed_sub(spat, srep, &s, global, use_bytes, use_utf8, loc),
                Matcher::Perl(re, srep) => perl_sub(re, srep, &s, global, use_utf8, i, loc, rep)?,
                Matcher::TreBytes(re, srep) => tre_sub(re, &s, srep, global, i, true, rep)?,
                Matcher::TreWide(..) => unreachable!(),
            };
            done.map(|(mut buf, offset)| {
                // copy the tail
                buf.extend_from_slice(&s[offset..]);
                Elt::Str(buf, new_mark(xi))
            })
        };
        ans.push(match made {
            None => mark_bytes_old(cx, xi, use_bytes, have_bytes_input),
            Some(_) if rep_na => Elt::Na,
            Some(e) => e,
        });
    }
    Ok(ans)
}

/// A code unit of a string: a byte, or a wide character.
trait Unit: Copy + Eq {
    fn code(self) -> u32;
    fn subj(s: &[Self]) -> Subj<'_>;
}

impl Unit for u8 {
    fn code(self) -> u32 {
        self as u32
    }
    fn subj(s: &[u8]) -> Subj<'_> {
        Subj::Bytes(s)
    }
}

impl Unit for i32 {
    fn code(self) -> u32 {
        self as u32
    }
    fn subj(s: &[i32]) -> Subj<'_> {
        Subj::Wide(s)
    }
}

/// `count_subs(repl)` / `wcount_subs(repl)`: how many `\1`..`\9` the
/// replacement has.
// R: grep.c:count_subs, grep.c:wcount_subs
fn count_subs<T: Unit>(repl: &[T]) -> usize {
    let (mut i, mut p) = (0, 0);
    while p < repl.len() {
        if repl[p].code() == b'\\' as u32 {
            match repl.get(p + 1).map(|c| c.code()) {
                Some(c) if (b'1' as u32..=b'9' as u32).contains(&c) => {
                    i += 1;
                    p += 2;
                }
                None => p += 1,
                Some(_) => p += 2,
            }
        } else {
            p += 1;
        }
    }
    i
}

/// `string_adj(target, orig, repl, regmatch)` / `wstring_adj`: the
/// replacement, with `\k` the text of group `k`, appended to `target`.
// R: grep.c:string_adj, grep.c:wstring_adj
fn string_adj<T: Unit>(target: &mut Vec<T>, orig: &[T], repl: &[T], regmatch: &[Match]) {
    let mut p = 0;
    while p < repl.len() {
        if repl[p].code() == b'\\' as u32 {
            match repl.get(p + 1).map(|c| c.code()) {
                Some(c) if (b'1' as u32..=b'9' as u32).contains(&c) => {
                    let k = (c - b'0' as u32) as usize;
                    let Match { so, eo } = regmatch[k];
                    if so < eo {
                        target.extend_from_slice(&orig[so as usize..eo as usize]);
                    }
                    p += 2;
                }
                None => p += 1,
                Some(_) => {
                    target.push(repl[p + 1]);
                    p += 2;
                }
            }
        } else {
            target.push(repl[p]);
            p += 1;
        }
    }
}

/// `sub_buffer_check_overflow(d)`: GNU R's buffer sizes must stay below
/// INT_MAX.
// R: grep.c:sub_buffer_check_overflow
fn check_overflow(d: f64) -> Result<(), Fail> {
    // 2147483647 is a length limit for R strings and 32-bit ints can be
    // precisely represented in IEEE double (but not 64-bit ints)
    if !(d < i32::MAX as f64) { Err(Fail::Error("result string is too long".into())) } else { Ok(()) }
}

/// `sub_buffer_size_init(replen, ns, nsubs, global, &nns, &maxrep)`'s
/// checks: `maxrep`, the most a replacement can add. (The buffer's size
/// itself is robigo's business; only GNU R's errors on it are kept.)
// R: grep.c:sub_buffer_size_init
fn buffer_init(replen: usize, ns: usize, nsubs: usize, global: bool) -> Result<f64, Fail> {
    // worst possible scenario is to put a copy of the replacement after
    // every character, unless there are backrefs
    let maxrep = replen as f64 + (ns as f64 - 2.) * nsubs as f64;
    check_overflow(maxrep)?;
    let maxrep = maxrep as i32 as f64;
    if global {
        let mut dnns = ns as f64 * (maxrep + 1.) + 1000.;
        if dnns > 10000. {
            dnns = 2. * ns as f64 + replen as f64 + 1000.;
        }
        check_overflow(dnns)?;
    } else {
        check_overflow(ns as f64 + maxrep + 1000.)?;
    }
    Ok(maxrep)
}

/// The check of `sub_buffer_expand` in the loop: the result so far, the
/// rest of the input and a replacement.
// R: grep.c:sub_buffer_size_expand
fn buffer_step(used: usize, rest: usize, maxrep: f64) -> Result<(), Fail> {
    check_overflow(used as f64 + rest as f64 + maxrep + 100.)
}

/// The fixed-string path: the result up to the unmatched tail and where
/// the tail starts, or `None` without a match.
// R: grep.c:do_gsub (fixed_opt)
fn fixed_sub(
    spat: &[u8],
    srep: &[u8],
    s: &[u8],
    global: bool,
    use_bytes: bool,
    use_utf8: bool,
    loc: Locale,
) -> Option<(Vec<u8>, usize)> {
    let patlen = spat.len();
    let mut st = fgrep_one_bytes(spat, s, use_bytes, use_utf8, loc)?;
    let mut u = Vec::with_capacity(s.len() + srep.len());
    let mut off = 0;
    loop {
        u.extend_from_slice(&s[off..off + st]);
        off += st + patlen;
        u.extend_from_slice(srep);
        if !global {
            break;
        }
        match fgrep_one_bytes(spat, &s[off..], use_bytes, use_utf8, loc) {
            Some(next) => st = next,
            None => break,
        }
    }
    Some((u, off))
}

/// The TRE path, on bytes (`tre_regexecb`) or wide characters
/// (`tre_regwexec`): the result up to the unmatched tail and where the tail
/// starts, or `None` without a match. Only the bytes loop reports
/// REG_ESPACE (`espace`).
// R: grep.c:do_gsub (!fixed_opt && !perl_opt)
fn tre_sub<T: Unit>(
    re: &Regex,
    s: &[T],
    srep: &[T],
    global: bool,
    i: usize,
    espace: bool,
    rep: &mut Report,
) -> Result<Option<(Vec<T>, usize)>, Fail> {
    let ns = s.len();
    let maxrep = buffer_init(srep.len(), ns, count_subs(srep), global)?;
    let mut u: Vec<T> = Vec::with_capacity(ns + srep.len());
    let (mut offset, mut nmatch, mut eflags, mut last_end) = (0usize, 0usize, 0, -1i64);
    let mut regmatch = [Match::UNSET; 10];
    let rc = loop {
        match tre_exec(re, T::subj(&s[offset..]), &mut regmatch, eflags)? {
            Ok(true) => {}
            Ok(false) => break Ok(()),
            Err(st) => break Err(st),
        }
        nmatch += 1;
        let (so, eo) = (regmatch[0].so as usize, regmatch[0].eo as usize);
        u.extend_from_slice(&s[offset..offset + so]);
        if (offset + eo) as i64 > last_end {
            string_adj(&mut u, &s[offset..], srep, &regmatch);
            last_end = (offset + eo) as i64;
        }
        offset += eo;
        if offset == ns || !global {
            break Ok(());
        }
        if eo == so {
            u.push(s[offset]);
            offset += 1;
        }
        buffer_step(u.len(), ns - offset, maxrep)?;
        eflags = tre::REG_NOTBOL;
    };
    // AFAICS the only possible error report is REG_ESPACE
    if espace && rc == Err(tre::Status::Espace) {
        rep.warn(format!("Out-of-memory error in regexp matching for element {}", i + 1));
    }
    tail(u, offset, ns, nmatch)
}

/// The end of a loop: no match, or the check of the tail's copy.
fn tail<T>(u: Vec<T>, offset: usize, ns: usize, nmatch: usize) -> Result<Option<(Vec<T>, usize)>, Fail> {
    if nmatch == 0 {
        return Ok(None);
    }
    // (GNU R checks only when it copies the tail, not for an NA
    // replacement; this bound is far beyond the loop's anyway)
    check_overflow(u.len() as f64 + (ns - offset) as f64 + 1.)?;
    Ok(Some((u, offset)))
}

/// The PCRE path: the result up to the unmatched tail and where the tail
/// starts, or `None` without a match.
// R: grep.c:do_gsub (perl_opt)
#[allow(clippy::too_many_arguments)]
fn perl_sub(
    re: &Pcre,
    srep: &[u8],
    s: &[u8],
    global: bool,
    use_utf8: bool,
    i: usize,
    loc: Locale,
    rep: &mut Report,
) -> Result<Option<(Vec<u8>, usize)>, Fail> {
    let ns = s.len();
    let maxrep = buffer_init(srep.len(), ns, count_subs(srep), global)?;
    let mut u = Vec::with_capacity(ns + srep.len());
    let (mut offset, mut nmatch, mut notbol, mut last_end) = (0usize, 0usize, false, None::<usize>);
    let mut locs = re.locations();
    let rc = loop {
        if use_utf8 && offset < ns && s[offset] & 0xc0 == 0x80 {
            // after a match ending inside a character (`\C`): GNU R goes on
            // with PCRE2_NO_UTF_CHECK, which the `pcre2` crate does not pass
            return Err(Fail::NotYet("a PCRE match from inside a UTF-8 character (gsub after '\\C')".into()));
        }
        match re.match_at(&mut locs, s, offset, notbol)? {
            Ok(true) => {}
            Ok(false) => break Ok(()),
            Err(msg) => break Err(msg),
        }
        let (so, eo) = locs.get(0).expect("robigo: a match");
        nmatch += 1;
        if offset < so {
            u.extend_from_slice(&s[offset..so]);
        }
        if last_end.is_none_or(|l| eo > l) {
            pcre_string_adj(&mut u, s, srep, &locs, use_utf8, loc);
            last_end = Some(eo);
        }
        offset = eo;
        if offset == ns || !global {
            break Ok(());
        }
        if eo == so {
            // advance by a char
            let used = if use_utf8 { utf8clen(s[offset]) } else { 1 };
            let pos = (offset + used).min(ns);
            u.extend_from_slice(&s[offset..pos]);
            offset = pos;
        }
        buffer_step(u.len(), ns - offset, maxrep)?;
        // probably not needed
        notbol = true;
    };
    if let Err(msg) = rc {
        rep.warn(pcre_exec_error(&msg, i));
    }
    tail(u, offset, ns, nmatch)
}

/// `R_pcre_string_adj(target, orig, repl, ovec, use_UTF8, ncap)`: the
/// replacement, with `\k` the text of group `k` (empty if it did not
/// match) and `\U`, `\L`, `\E` changing the case of what follows.
// R: grep.c:R_pcre_string_adj
fn pcre_string_adj(
    t: &mut Vec<u8>,
    orig: &[u8],
    repl: &[u8],
    locs: &pcre2::bytes::CaptureLocations,
    use_utf8: bool,
    loc: Locale,
) {
    let (mut upper, mut lower) = (false, false);
    let mut p = 0;
    while p < repl.len() {
        if repl[p] != b'\\' {
            t.push(repl[p]);
            p += 1;
            continue;
        }
        match repl.get(p + 1) {
            Some(&c @ b'1'..=b'9') => {
                // a back-reference to a group that has not been captured is
                // an empty string
                if let Some((so, eo)) = locs.get((c - b'0') as usize) {
                    let g = &orig[so..eo.max(so)];
                    if !g.is_empty() && use_utf8 && (upper || lower) {
                        // Here we need to work in chars
                        if let Ok(g) = std::str::from_utf8(g) {
                            let l = ctype(loc);
                            for ch in g.chars() {
                                let w = if upper { tre::towupper(ch as u32, l) } else { tre::towlower(ch as u32, l) };
                                let ch = char::from_u32(w).unwrap_or(ch);
                                t.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                            }
                        }
                    } else {
                        // toupper/tolower of a char: the C and UTF-8 locales
                        // change only ASCII
                        t.extend(g.iter().map(|&b| {
                            if upper {
                                b.to_ascii_uppercase()
                            } else if lower {
                                b.to_ascii_lowercase()
                            } else {
                                b
                            }
                        }));
                    }
                }
                p += 2;
            }
            Some(b'U') => {
                p += 2;
                (upper, lower) = (true, false);
            }
            Some(b'L') => {
                p += 2;
                (upper, lower) = (false, true);
            }
            // end case modification
            Some(b'E') => {
                p += 2;
                (upper, lower) = (false, false);
            }
            None => p += 1,
            Some(&c) => {
                t.push(c);
                p += 2;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_adjusts_backrefs() {
        assert_eq!(count_subs(b"a\\1b\\2\\x\\"), 2);
        let mut t = Vec::new();
        let m = [Match { so: 0, eo: 3 }, Match { so: 1, eo: 2 }, Match::UNSET, Match::UNSET];
        let mut regmatch = [Match::UNSET; 10];
        regmatch[..4].copy_from_slice(&m);
        string_adj(&mut t, b"abc", b"<\\1|\\2|\\0\\", &regmatch);
        assert_eq!(t, b"<b||0");
    }
}
