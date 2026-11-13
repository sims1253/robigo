//! sprintf.c: `.Internal(sprintf(fmt, ...))`, with each conversion done as
//! glibc's `snprintf` does it ([`robigo_text::printf`]).

use crate::coerce::vector::coerce;
use crate::util::{not_yet, type2char};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Encoding, Out, RResult, Ref, Root, Scope, View};
use robigo_text::printf::Spec;

/// `MAXLINE` (`MAXELTSIZE`): the longest format and conversion.
const MAXLINE: usize = 8192;
/// `MAXNARGS`.
const MAXNARGS: usize = 100;

/// `findspec(str)`: past `%` and the flags, width and precision.
fn findspec(f: &[u8]) -> usize {
    if f.first() != Some(&b'%') {
        return 0;
    }
    let mut p = 1;
    while p < f.len() && (b"-+ #.*".contains(&f[p]) || f[p].is_ascii_digit()) {
        p += 1;
    }
    p
}

/// `checkfmt(fmt, pattern)`: true (an error) unless the conversion is one
/// of `pattern`.
fn checkfmt(f: &[u8], pattern: &[u8]) -> bool {
    if f.first() != Some(&b'%') {
        return true;
    }
    let p = findspec(f);
    !(p < f.len() && pattern.contains(&f[p]))
}

/// `snprintf(bit, MAXLINE+1, fmt, ...)` through the parsed spec, with GNU
/// R's length error.
fn conv(cx: &mut Cx, out: &mut Vec<u8>, fmt: &[u8], f: impl FnOnce(&Spec, &mut Vec<u8>)) -> RResult<()> {
    let Some(spec) = Spec::parse(fmt) else {
        let msg = format!("unrecognised format specification '{}'", String::from_utf8_lossy(fmt));
        return Err(cx.error(&msg));
    };
    let mut bit = Vec::new();
    f(&spec, &mut bit);
    if bit.len() > MAXLINE {
        let msg = format!("required resulting string length {} is greater than maximal {}", bit.len(), MAXLINE);
        return Err(cx.error(&msg));
    }
    out.extend_from_slice(&bit);
    Ok(())
}

/// `.Internal(sprintf(fmt, ...))`.
// R: sprintf.c:do_sprintf
pub fn do_sprintf(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    if args.is_empty() || args.get(cx, 0).sexptype() != STRSXP {
        return Err(cx.error("'fmt' is not a character vector"));
    }
    let nfmt = args.get(cx, 0).len().unwrap_or(0);
    if nfmt == 0 {
        return cx.build(out, |cx| Ok(cx.alloc_vector::<Char<'_>>(0)?.finish().into()));
    }
    let nargs = args.len() - 1;
    if nargs >= MAXNARGS {
        return Err(cx.error(&format!("only {MAXNARGS} arguments are allowed")));
    }
    if args.any_package_altrep() {
        return Err(not_yet(cx, Some(call), "sprintf() of a package ALTREP vector"));
    }
    cx.scope(|cx, s| {
        let mut a: Vec<Root<'_>> = (1..=nargs).map(|i| args.root(i)).collect();
        let mut lens = vec![0usize; nargs];
        for i in 0..nargs {
            let t = a[i].bind(cx).sexptype();
            if t == LANGSXP || t == SYMSXP {
                let msg = format!("invalid type of argument[{}]: '{}'", i + 1, type2char(t));
                return Err(cx.error(&msg));
            }
            lens[i] = crate::coerce::xlength(a[i].bind(cx));
            if lens[i] == 0 {
                return cx.build(out, |cx| Ok(cx.alloc_vector::<Char<'_>>(0)?.finish().into()));
            }
        }
        let check_maxlen = |cx: &mut Cx, lens: &[usize]| -> RResult<usize> {
            let maxlen = lens.iter().copied().fold(nfmt, usize::max);
            if maxlen % nfmt != 0 || lens.iter().any(|&l| maxlen % l != 0) {
                return Err(cx.error("arguments cannot be recycled to the same length"));
            }
            Ok(maxlen)
        };
        let mut maxlen = check_maxlen(cx, &lens)?;
        let mut used = vec![false; nargs];
        let loc = cx.locale();
        let mut rows: Vec<(Vec<u8>, bool)> = Vec::new();
        let mut ns = 0;
        while ns < maxlen {
            let (format, use_utf8) = {
                let fv = args.get(cx, 0).as_vector::<Char<'_>>().unwrap();
                let fc = fv.get(ns % nfmt);
                let mut u = fc.enc() == Encoding::Utf8;
                if !u {
                    for i in 0..nargs {
                        if let View::Str(v) = a[i].bind(cx).view()
                            && v.get(ns % lens[i]).enc() == Encoding::Utf8
                        {
                            u = true;
                            break;
                        }
                    }
                }
                (tr(cx, fc, u), u)
            };
            let n = format.len();
            if n > MAXLINE {
                return Err(cx.error(&format!("'fmt' length exceeds maximal format length {MAXLINE}")));
            }
            let mut output: Vec<u8> = Vec::new();
            let mut cnt = 0usize;
            let mut cur = 0usize;
            while cur < n {
                let cf = &format[cur..];
                if format[cur] != b'%' {
                    let chunk = cf.iter().position(|&c| c == b'%').unwrap_or(cf.len());
                    output.extend_from_slice(&cf[..chunk]);
                    cur += chunk;
                    continue;
                }
                if cur < n - 1 && format[cur + 1] == b'%' {
                    output.push(b'%');
                    cur += 2;
                    continue;
                }
                let chunk = cf[1..].iter().position(|c| b"diosfeEgGxXaA".contains(c)).unwrap_or(cf.len() - 1) + 2;
                if cur + chunk > n {
                    let msg = format!("unrecognised format specification '{}'", String::from_utf8_lossy(cf));
                    return Err(cx.error(&msg));
                }
                let mut fmt = cf[..chunk].to_vec();
                cur += chunk;
                // %n$ and %nn$
                let mut nthis: Option<usize> = None;
                if fmt.len() > 3 && (b'1'..=b'9').contains(&fmt[1]) {
                    let v1 = (fmt[1] - b'0') as usize;
                    if fmt[2] == b'$' {
                        if v1 > nargs {
                            return Err(cx.error(&format!("reference to non-existent argument {v1}")));
                        }
                        nthis = Some(v1 - 1);
                        fmt.drain(1..3);
                    } else if fmt[2].is_ascii_digit() && fmt[3] == b'$' {
                        let v = 10 * v1 + (fmt[2] - b'0') as usize;
                        if v > nargs {
                            return Err(cx.error(&format!("reference to non-existent argument {v}")));
                        }
                        nthis = Some(v - 1);
                        fmt.drain(1..4);
                    }
                }
                // `*`
                let mut star: Option<i32> = None;
                if let Some(sp) = fmt.iter().position(|&c| c == b'*') {
                    let mut nstar: Option<usize> = None;
                    let rest = &fmt[sp..];
                    if rest.len() > 3 && (b'1'..=b'9').contains(&rest[1]) {
                        let v1 = (rest[1] - b'0') as usize;
                        if rest[2] == b'$' {
                            if v1 > nargs {
                                return Err(cx.error(&format!("reference to non-existent argument {v1}")));
                            }
                            nstar = Some(v1 - 1);
                            fmt.drain(sp + 1..sp + 3);
                        } else if rest[2].is_ascii_digit() && rest[3] == b'$' {
                            let v = 10 * v1 + (rest[2] - b'0') as usize;
                            if v > nargs {
                                return Err(cx.error(&format!("reference to non-existent argument {v}")));
                            }
                            nstar = Some(v - 1);
                            fmt.drain(sp + 1..sp + 4);
                        }
                    }
                    let k = match nstar {
                        Some(k) => k,
                        None => {
                            if cnt >= nargs {
                                return Err(cx.error("too few arguments"));
                            }
                            cnt += 1;
                            cnt - 1
                        }
                    };
                    if fmt[sp + 1..].contains(&b'*') {
                        return Err(cx.error(
                            "at most one asterisk '*' is supported in each conversion specification",
                        ));
                    }
                    used[k] = true;
                    if ns == 0 && a[k].bind(cx).sexptype() == REALSXP {
                        let r = coerced(cx, s, a[k], INTSXP)?;
                        a[k] = r;
                    }
                    let t = a[k].bind(cx);
                    let v = match t.as_vector::<i32>() {
                        Some(v) if t.sexptype() == INTSXP && !v.is_empty() => v.get(ns % v.len()),
                        _ => i32::MIN,
                    };
                    if v == i32::MIN {
                        return Err(cx.error("argument for '*' conversion specification must be a number"));
                    }
                    star = Some(v);
                }
                if fmt.last() == Some(&b'%') {
                    // "%5%" and the like: copied as written (or with `*`)
                    if star.is_some() {
                        return Err(not_yet(cx, Some(call), "sprintf() of a '%' conversion with a '*' width"));
                    }
                    output.extend_from_slice(&fmt);
                    continue;
                }
                let k = match nthis {
                    Some(k) => k,
                    None => {
                        if cnt >= nargs {
                            return Err(cx.error("too few arguments"));
                        }
                        cnt += 1;
                        cnt - 1
                    }
                };
                used[k] = true;
                let mut fmtp = match star {
                    Some(w) => expand_star(&fmt, w),
                    None => fmt.clone(),
                };
                if fmtp.len() > MAXLINE {
                    return Err(cx.error(&format!("'fmt' length exceeds maximal format length {MAXLINE}")));
                }
                // minimal coercion, once, for the first row
                if ns == 0 {
                    let c = fmtp.get(findspec(&fmtp)).copied().unwrap_or(0);
                    let t = a[k].bind(cx).sexptype();
                    match c {
                        b'd' | b'i' | b'o' | b'x' | b'X' if t == REALSXP => {
                            let exact = a[k].bind(cx).as_vector::<f64>().unwrap().as_slice().unwrap().iter().all(|&r| {
                                robigo_text::is_na(r) || (r.is_finite() && (r as i32) as f64 == r)
                            });
                            if exact {
                                let r = coerced(cx, s, a[k], INTSXP)?;
                                a[k] = r;
                            }
                        }
                        b'a' | b'A' | b'e' | b'f' | b'g' | b'E' | b'G' if t != REALSXP && t != STRSXP => {
                            a[k] = through_r(cx, s, b"as.double", a[k], env)?;
                            maxlen = relength(cx, &mut lens, k, a[k], maxlen, &check_maxlen)?;
                        }
                        b's' if t != STRSXP => {
                            a[k] = through_r(cx, s, b"as.character", a[k], env)?;
                            maxlen = relength(cx, &mut lens, k, a[k], maxlen, &check_maxlen)?;
                        }
                        _ => {}
                    }
                }
                let thislen = crate::coerce::xlength(a[k].bind(cx));
                if thislen == 0 {
                    return Err(cx.error("coercion has changed vector length to 0"));
                }
                let this = a[k].bind(cx);
                match this.view() {
                    View::Lgl(v) => {
                        let x = v.get(ns % thislen).0;
                        if checkfmt(&fmtp, b"di") {
                            return Err(invalid_format(cx, &fmtp, "use format %d or %i for logical objects"));
                        }
                        if x == i32::MIN {
                            *fmtp.last_mut().unwrap() = b's';
                            conv(cx, &mut output, &fmtp, |sp, o| sp.string(o, b"NA"))?;
                        } else {
                            conv(cx, &mut output, &fmtp, |sp, o| sp.int(o, x))?;
                        }
                    }
                    View::Int(v) => {
                        let x = v.get(ns % thislen);
                        if checkfmt(&fmtp, b"dioxX") {
                            return Err(invalid_format(
                                cx,
                                &fmtp,
                                "use format %d, %i, %o, %x or %X for integer objects",
                            ));
                        }
                        if x == i32::MIN {
                            *fmtp.last_mut().unwrap() = b's';
                            conv(cx, &mut output, &fmtp, |sp, o| sp.string(o, b"NA"))?;
                        } else {
                            conv(cx, &mut output, &fmtp, |sp, o| sp.int(o, x))?;
                        }
                    }
                    View::Real(v) => {
                        let x = v.get(ns % thislen);
                        if checkfmt(&fmtp, b"aAfeEgG") {
                            return Err(invalid_format(cx, &fmtp, "use format %f, %e, %g or %a for numeric objects"));
                        }
                        let c = fmtp[findspec(&fmtp)];
                        if x.is_finite() {
                            if c == b'a' || c == b'A' {
                                return Err(not_yet(cx, Some(call), "sprintf() with %a (hexadecimal floating point)"));
                            }
                            conv(cx, &mut output, &fmtp, |sp, o| sp.double(o, x))?;
                        } else {
                            match fmtp.iter().position(|&ch| ch == b'.') {
                                Some(p) => {
                                    fmtp.truncate(p);
                                    fmtp.push(b's');
                                }
                                None => *fmtp.last_mut().unwrap() = b's',
                            }
                            let has = |ch: u8| fmtp.contains(&ch);
                            let txt: &[u8] = if robigo_text::is_na(x) {
                                if has(b' ') { b" NA" } else { b"NA" }
                            } else if x.is_nan() {
                                if has(b' ') { b" NaN" } else { b"NaN" }
                            } else if x > 0.0 {
                                if has(b'+') {
                                    b"+Inf"
                                } else if has(b' ') {
                                    b" Inf"
                                } else {
                                    b"Inf"
                                }
                            } else {
                                b"-Inf"
                            };
                            conv(cx, &mut output, &fmtp, |sp, o| sp.string(o, txt))?;
                        }
                    }
                    View::Str(v) => {
                        if checkfmt(&fmtp, b"s") {
                            return Err(invalid_format(cx, &fmtp, "use format %s for character objects"));
                        }
                        let ss = tr(cx, v.get(ns % thislen), use_utf8);
                        if fmtp.get(1) != Some(&b's') {
                            if ss.len() > MAXLINE {
                                cx.warning(&format!(
                                    "likely truncation of character string to {} characters",
                                    MAXLINE - 1
                                ))?;
                            }
                            conv(cx, &mut output, &fmtp, |sp, o| sp.string(o, &ss))?;
                        } else {
                            output.extend_from_slice(&ss);
                        }
                    }
                    _ => return Err(cx.error("unsupported type")),
                }
            }
            rows.push((output, use_utf8));
            ns += 1;
        }
        let nunused = used.iter().filter(|u| !**u).count();
        if nunused > 0 {
            let msg = if nfmt == 1 {
                let f = {
                    let fc = args.get(cx, 0).as_vector::<Char<'_>>().unwrap().get(0);
                    String::from_utf8_lossy(&tr(cx, fc, false)).into_owned()
                };
                if nunused == 1 {
                    format!("one argument not used by format '{f}'")
                } else {
                    format!("{nunused} arguments not used by format '{f}'")
                }
            } else if nunused == 1 {
                "one argument not used by format".to_owned()
            } else {
                format!("{nunused} arguments not used by format")
            };
            cx.build(out, |cx| strings(cx, &rows))?;
            return cx.warning(&msg);
        }
        let _ = loc;
        cx.build(out, |cx| strings(cx, &rows))
    })
}

/// The rows as a character vector (`mkCharCE(row, use_UTF8 ? CE_UTF8 :
/// CE_NATIVE)`).
fn strings<'g>(cx: &'g Cx, rows: &[(Vec<u8>, bool)]) -> Result<Ref<'g>, robigo_rtcore::NeedGc> {
    let mut v = cx.alloc_vector::<Char<'g>>(rows.len())?;
    for (i, (b, u)) in rows.iter().enumerate() {
        v.set(i, cx.mk_char(b, if *u { Encoding::Utf8 } else { Encoding::Native })?);
    }
    Ok(v.finish().into())
}

/// `TRANSLATE_CHAR(x, i)`: UTF-8 or native bytes of a CHARSXP.
fn tr(cx: &Cx, c: Char<'_>, utf8: bool) -> Vec<u8> {
    let s = crate::coerce::text(c);
    let r = if utf8 { crate::text::to_utf8(s, cx.locale()) } else { crate::text::to_native(s, cx.locale()) };
    r.map(|b| b.into_owned()).unwrap_or_else(|_| c.bytes().to_vec())
}

/// `fmt` with its `*` replaced by the width `w`.
fn expand_star(fmt: &[u8], w: i32) -> Vec<u8> {
    let mut f = Vec::with_capacity(fmt.len() + 8);
    for &c in fmt {
        if c == b'*' {
            f.extend_from_slice(w.to_string().as_bytes());
        } else {
            f.push(c);
        }
    }
    f
}

/// GNU R's "invalid format '%s'; %s".
fn invalid_format(cx: &mut Cx, fmt: &[u8], hint: &str) -> robigo_rtcore::RError {
    cx.error(&format!("invalid format '{}'; {hint}", String::from_utf8_lossy(fmt)))
}

/// `coerceVector(x, ty)`, rooted in `s`.
fn coerced<'s>(cx: &mut Cx, s: Scope<'s>, x: Root<'_>, ty: u8) -> RResult<Root<'s>> {
    let o = s.out(cx);
    let r = o.root();
    let w = cx.build_with(o, |cx| coerce(cx, x.bind(cx), ty, true))?;
    w.signal(cx)?;
    Ok(r)
}

/// `eval(lang2(install(f), x), env)`, rooted in `s`.
fn through_r<'s>(cx: &mut Cx, s: Scope<'s>, f: &[u8], x: Root<'_>, env: Root<'_>) -> RResult<Root<'s>> {
    let call = s.capture(cx, |cx, o| {
        cx.build(o, |cx| {
            let f = cx.install(f)?;
            let a = cx.cons(x.bind(cx), cx.nil())?;
            cx.lcons(f.into(), a)
        })
    })?;
    s.capture(cx, |cx, o| cx.eval(call, env, o))
}

/// `CHECK_this_length` after a coercion through R, and `CHECK_maxlen` if
/// the argument set the length.
fn relength(
    cx: &mut Cx,
    lens: &mut [usize],
    k: usize,
    v: Root<'_>,
    maxlen: usize,
    check: &dyn Fn(&mut Cx, &[usize]) -> RResult<usize>,
) -> RResult<usize> {
    let thislen = crate::coerce::xlength(v.bind(cx));
    if thislen == 0 {
        return Err(cx.error("coercion has changed vector length to 0"));
    }
    let do_check = lens[k] == maxlen;
    lens[k] = thislen;
    if do_check && thislen < maxlen {
        return check(cx, lens);
    }
    Ok(maxlen)
}
