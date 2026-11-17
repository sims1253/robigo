//! `.Internal(grep(pattern, x, ignore.case, value, perl, fixed, useBytes,
//! invert))` and `grepl`.

use super::{
    Fail, Pcre, Report, Subj, Text, fgrep_one, have, input, only_ascii, pattern, pcre_exec_error, strings, tre_exec,
    tre_fail, wide_input, wtrans,
};
use crate::attrib::{self, Attr, Attrs};
use crate::util::as_bool2;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Built, Char, Cx, Encoding, Lgl, Locale, Out, RResult, RSym, Ref, Root};
use robigo_text::tre::{self, Regex};

/// How the pattern matches.
enum Matcher {
    Fixed(Vec<u8>),
    Perl(Pcre),
    Tre(Regex),
}

/// `grep` (`CODE` 0) and `grepl` (1).
// R: grep.c:do_grep
pub fn do_grep<const CODE: u8>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let grepl = CODE == 1;
    let row = if grepl { crate::row!("grepl", true) } else { crate::row!("grep", true) };
    row.check_arity(cx, call, args)?;
    let pat = args.root(0);
    let text = args.root(1);
    let igcase_opt = as_bool2(cx, call, args.root(2))?;
    let value_opt = as_bool2(cx, call, args.root(3))?;
    let mut perl_opt = as_bool2(cx, call, args.root(4))?;
    let fixed_opt = as_bool2(cx, call, args.root(5))?;
    let use_bytes = as_bool2(cx, call, args.root(6))?;
    let invert = as_bool2(cx, call, args.root(7))?;
    if fixed_opt && igcase_opt {
        cx.warning("argument 'ignore.case = TRUE' will be ignored")?;
    }
    if fixed_opt && perl_opt {
        cx.warning("argument 'perl = TRUE' will be ignored")?;
        perl_opt = false;
    }
    let (pt, pn) = {
        let p = pat.bind(cx);
        (p.sexptype(), if p.sexptype() == STRSXP { strings(p).len() } else { 0 })
    };
    if pt != STRSXP || pn < 1 {
        return Err(cx.error("invalid 'pattern' argument"));
    }
    if pn > 1 {
        cx.warning("argument 'pattern' has length > 1 and only the first element will be used")?;
    }
    if text.bind(cx).sexptype() != STRSXP {
        return Err(cx.error("invalid 'text' argument"));
    }
    let n = strings(text.bind(cx)).len();
    let loc = cx.locale();
    let p0 = strings(pat.bind(cx)).get(0);
    let ind: Vec<bool> = if p0 == cx.na_string() {
        return na_result(cx, out, text, n, grepl, value_opt);
    } else {
        let mut rep = Report::default();
        let r = {
            let cx: &Cx = cx;
            let tv = strings(text.bind(cx));
            let xs: Vec<Char<'_>> = (0..n).map(|i| tv.get(i)).collect();
            let o = Opts { igcase: igcase_opt, perl: perl_opt, fixed: fixed_opt, use_bytes, loc };
            matches(cx, p0, &xs, o, &mut rep)
        };
        rep.finish(cx, call, r)?
    };
    let nmatches = ind.iter().filter(|&&m| invert ^ m).count();
    cx.build_with(out, |cx| {
        let tv = text.bind(cx);
        if grepl {
            let mut v = cx.alloc_vector::<Lgl>(n)?;
            for (d, &m) in v.as_mut_slice().iter_mut().zip(&ind) {
                *d = if m { Lgl::TRUE } else { Lgl::FALSE };
            }
            return Ok(Built::done(v.finish()));
        }
        let sel = || ind.iter().enumerate().filter(|&(_, &m)| invert ^ m).map(|(i, _)| i);
        if value_opt {
            let xs = strings(tv);
            let mut ans = cx.alloc_vector::<Char<'_>>(nmatches)?;
            for (j, i) in sel().enumerate() {
                ans.set(j, xs.get(i));
            }
            // copy across names and subset
            if let Some(nmold) = attrib::names(tv) {
                let old = strings(nmold);
                let mut nm = cx.alloc_vector::<Char<'_>>(nmatches)?;
                for (j, i) in sel().enumerate() {
                    nm.set(j, old.get(i));
                }
                let nm: Ref<'_> = nm.finish().into();
                let mut a = Attrs::new();
                a.install(Attr { tag: cx.sym(RSym::Names), value: nm });
                a.apply(cx, &mut ans)?;
            }
            Ok(Built::done(ans.finish()))
        } else if n > i32::MAX as usize {
            let mut v = cx.alloc_vector::<f64>(nmatches)?;
            for (d, i) in v.as_mut_slice().iter_mut().zip(sel()) {
                *d = (i + 1) as f64;
            }
            Ok(Built::done(v.finish()))
        } else {
            let mut v = cx.alloc_vector::<i32>(nmatches)?;
            for (d, i) in v.as_mut_slice().iter_mut().zip(sel()) {
                *d = (i + 1) as i32;
            }
            Ok(Built::done(v.finish()))
        }
    })
}

/// The result for `pattern = NA`: all NA.
fn na_result(cx: &mut Cx, out: Out<'_>, text: Root<'_>, n: usize, grepl: bool, value_opt: bool) -> RResult<()> {
    cx.build_with(out, |cx| {
        if value_opt {
            let mut ans = cx.alloc_vector::<Char<'_>>(n)?;
            for i in 0..n {
                ans.set(i, cx.na_string());
            }
            if let Some(nmold) = attrib::names(text.bind(cx)) {
                // (a duplicate: the names are shared copy-on-modify)
                let mut a = Attrs::new();
                a.install(Attr { tag: cx.sym(RSym::Names), value: nmold });
                a.apply(cx, &mut ans)?;
            }
            Ok(Built::done(ans.finish()))
        } else if grepl {
            let mut v = cx.alloc_vector::<Lgl>(n)?;
            v.as_mut_slice().fill(Lgl::NA);
            Ok(Built::done(v.finish()))
        } else {
            let mut v = cx.alloc_vector::<i32>(n)?;
            v.as_mut_slice().fill(i32::MIN);
            Ok(Built::done(v.finish()))
        }
    })
}

#[derive(Clone, Copy)]
struct Opts {
    igcase: bool,
    perl: bool,
    fixed: bool,
    use_bytes: bool,
    loc: Locale,
}

/// Phase 2: whether each string matches.
fn matches(cx: &Cx, p0: Char<'_>, xs: &[Char<'_>], o: Opts, rep: &mut Report) -> Result<Vec<bool>, Fail> {
    let Opts { igcase, perl, fixed, loc, .. } = o;
    let pats = [p0];
    let mut use_bytes = o.use_bytes;
    if !use_bytes {
        use_bytes = only_ascii(cx, &pats) && only_ascii(cx, xs);
    }
    if !use_bytes {
        use_bytes = have(&pats, Encoding::Bytes) || have(xs, Encoding::Bytes);
    }
    let mut use_utf8 = false;
    if !use_bytes {
        // As from R 2.10.0 we use UTF-8 mode in PCRE in all MBCS locales
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
    let what = || "regular expression".to_string();
    let m = if use_wc {
        let Some(wpat) = wtrans(p0, loc, rep) else {
            return Err(Fail::Error("regular expression is invalid".into()));
        };
        let cflags = tre::REG_NOSUB | tre::REG_EXTENDED | if igcase { tre::REG_ICASE } else { 0 };
        Matcher::Tre(Regex::new(&wpat, cflags, crate::text::ctype(loc)).map_err(|e| tre_fail(e, None, rep))?)
    } else {
        let spat = pattern(p0, what, use_bytes, use_utf8, loc, rep)?;
        if fixed {
            Matcher::Fixed(spat.into_owned())
        } else if perl {
            Matcher::Perl(Pcre::prepare(&spat, use_utf8, igcase, loc, rep)?)
        } else {
            let cflags = tre::REG_NOSUB | tre::REG_EXTENDED | if igcase { tre::REG_ICASE } else { 0 };
            Matcher::Tre(
                Regex::new_bytes(&spat, cflags, crate::text::ctype(loc))
                    .map_err(|e| tre_fail(e, Some(&String::from_utf8_lossy(&spat)), rep))?,
            )
        }
    };
    let mut ind = vec![false; xs.len()];
    for (i, &xi) in xs.iter().enumerate() {
        if xi == cx.na_string() {
            continue;
        }
        let s = if use_wc {
            match wide_input(xi, i, loc, rep) {
                Some(w) => Text::Wide(w),
                None => continue,
            }
        } else {
            match input(xi, i, use_bytes, use_utf8, loc, rep) {
                Some(b) => Text::Bytes(b),
                None => continue,
            }
        };
        ind[i] = match (&m, s.subj()) {
            (Matcher::Fixed(p), Subj::Bytes(b)) => fgrep_one(p, b, use_bytes, use_utf8, loc).is_some(),
            (Matcher::Perl(re), Subj::Bytes(b)) => match re.is_match(b) {
                Ok(m) => m,
                Err(msg) => {
                    rep.warn(pcre_exec_error(&msg, i));
                    false
                }
            },
            (Matcher::Tre(re), s) => match tre_exec(re, s, &mut [], 0)? {
                Ok(m) => m,
                Err(st) => {
                    // AFAICS the only possible error report is REG_ESPACE
                    if st == tre::Status::Espace {
                        rep.warn(format!("Out-of-memory error in regexp matching for element {}", i + 1));
                    }
                    false
                }
            },
            _ => unreachable!("robigo: fixed and PCRE matching are on bytes"),
        };
    }
    Ok(ind)
}
