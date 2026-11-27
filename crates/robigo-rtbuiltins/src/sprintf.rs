//! sprintf.c: `.Internal(sprintf(fmt, ...))`, with each conversion done as
//! glibc's `snprintf` does it ([`robigo_text::printf`]).

use crate::coerce::vector::coerce;
use crate::util::{not_yet, type2char};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Diag, Encoding, LARGE, NeedGc, Out, RResult, Ref, Root, Scope, View};
use robigo_text::printf::Spec;
use std::borrow::Cow;

/// `MAXLINE` (`MAXELTSIZE`): the longest format and conversion.
const MAXLINE: usize = 8192;
/// `MAXNARGS`.
const MAXNARGS: usize = 100;

/// `mk_char` upholding H's Rule 1 for `fill_strings`: refuse a `LARGE`
/// (`>= 1 MiB`) CHARSXP request while a collection is due so the fill serves
/// the safepoint before allocating it.
#[inline]
fn mk_char_checked<'g>(cx: &'g Cx, bytes: &[u8], enc: Encoding) -> Result<Char<'g>, NeedGc> {
    if bytes.len() >= LARGE && cx.gc_due() {
        return Err(NeedGc::new(CHARSXP, bytes.len()));
    }
    cx.mk_char(bytes, enc)
}

/// GNU R's "robigo: ... is not implemented yet" as a [`Diag`] against `call`.
fn not_yet_diag<'r>(call: Root<'r>, what: &str) -> Diag<'r> {
    Diag::error_call(Some(call), format!("robigo: {what} is not implemented yet"))
}

/// GNU R's truncation warning message for character strings longer than `MAXLINE`.
fn trunc_warn_msg() -> String {
    format!("likely truncation of character string to {} characters", MAXLINE - 1)
}

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
/// R's length error, reusing `bit`.
fn conv_diag<'r>(
    out: &mut Vec<u8>,
    bit: &mut Vec<u8>,
    fmt: &[u8],
    f: impl FnOnce(&Spec, &mut Vec<u8>),
) -> Result<(), Diag<'r>> {
    let Some(spec) = Spec::parse(fmt) else {
        return Err(Diag::error(format!("unrecognised format specification '{}'", String::from_utf8_lossy(fmt))));
    };
    bit.clear();
    f(&spec, bit);
    if bit.len() > MAXLINE {
        return Err(Diag::error(format!(
            "required resulting string length {} is greater than maximal {}",
            bit.len(),
            MAXLINE
        )));
    }
    out.extend_from_slice(bit);
    Ok(())
}

/// Scratch buffers reused across rows of `do_sprintf`'s `fill_strings` loop.
struct RowBufs {
    output: Vec<u8>,
    fmt: Vec<u8>,
    fmtp: Vec<u8>,
    bit: Vec<u8>,
}

impl RowBufs {
    fn new() -> Self {
        Self { output: Vec::new(), fmt: Vec::new(), fmtp: Vec::new(), bit: Vec::new() }
    }
}

/// Parse an optional `%n$` / `%nn$` argument index starting at `pos` inside
/// `fmt`, stripping it from `fmt` if present.
fn strip_arg_index<'r>(fmt: &mut Vec<u8>, pos: usize, nargs: usize) -> Result<Option<usize>, Diag<'r>> {
    let rest = &fmt[pos..];
    if rest.len() > 3 && (b'1'..=b'9').contains(&rest[1]) {
        let v1 = (rest[1] - b'0') as usize;
        if rest[2] == b'$' {
            if v1 > nargs {
                return Err(Diag::error(format!("reference to non-existent argument {v1}")));
            }
            fmt.drain(pos + 1..pos + 3);
            return Ok(Some(v1 - 1));
        } else if rest[2].is_ascii_digit() && rest[3] == b'$' {
            let v = 10 * v1 + (rest[2] - b'0') as usize;
            if v > nargs {
                return Err(Diag::error(format!("reference to non-existent argument {v}")));
            }
            fmt.drain(pos + 1..pos + 4);
            return Ok(Some(v - 1));
        }
    }
    Ok(None)
}

/// Format one already-coerced argument `this` at row `ns` into `output`.
#[allow(clippy::too_many_arguments)]
fn format_value<'r>(
    cx: &Cx,
    call: Root<'r>,
    this: Ref<'_>,
    ns: usize,
    thislen: usize,
    fmtp: &mut Vec<u8>,
    use_utf8: bool,
    output: &mut Vec<u8>,
    bit: &mut Vec<u8>,
    trunc_warns: &mut usize,
) -> Result<(), Diag<'r>> {
    match this.view() {
        View::Lgl(v) => {
            let x = v.get(ns % thislen).0;
            if checkfmt(fmtp, b"di") {
                return Err(invalid_format_diag(fmtp, "use format %d or %i for logical objects"));
            }
            if x == i32::MIN {
                *fmtp.last_mut().unwrap() = b's';
                conv_diag(output, bit, fmtp, |sp, o| sp.string(o, b"NA"))?;
            } else {
                conv_diag(output, bit, fmtp, |sp, o| sp.int(o, x))?;
            }
        }
        View::Int(v) => {
            let x = v.get(ns % thislen);
            if checkfmt(fmtp, b"dioxX") {
                return Err(invalid_format_diag(fmtp, "use format %d, %i, %o, %x or %X for integer objects"));
            }
            if x == i32::MIN {
                *fmtp.last_mut().unwrap() = b's';
                conv_diag(output, bit, fmtp, |sp, o| sp.string(o, b"NA"))?;
            } else {
                conv_diag(output, bit, fmtp, |sp, o| sp.int(o, x))?;
            }
        }
        View::Real(v) => {
            let x = v.get(ns % thislen);
            if checkfmt(fmtp, b"aAfeEgG") {
                return Err(invalid_format_diag(fmtp, "use format %f, %e, %g or %a for numeric objects"));
            }
            let c = fmtp[findspec(fmtp)];
            if x.is_finite() {
                if c == b'a' || c == b'A' {
                    return Err(not_yet_diag(call, "sprintf() with %a (hexadecimal floating point)"));
                }
                conv_diag(output, bit, fmtp, |sp, o| sp.double(o, x))?;
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
                conv_diag(output, bit, fmtp, |sp, o| sp.string(o, txt))?;
            }
        }
        View::Str(v) => {
            if checkfmt(fmtp, b"s") {
                return Err(invalid_format_diag(fmtp, "use format %s for character objects"));
            }
            let ss = tr_cow(cx, v.get(ns % thislen), use_utf8);
            if fmtp.get(1) != Some(&b's') {
                if ss.len() > MAXLINE {
                    *trunc_warns += 1;
                }
                conv_diag(output, bit, fmtp, |sp, o| sp.string(o, &ss))?;
            } else {
                output.extend_from_slice(&ss);
            }
        }
        _ => return Err(Diag::error("unsupported type")),
    }
    Ok(())
}

/// Whether row `ns` uses UTF-8 output encoding (`use_UTF8` in `sprintf.c`).
fn row_use_utf8(cx: &Cx, fc: Char<'_>, a: &[Root<'_>], lens: &[usize], ns: usize) -> bool {
    if fc.enc() == Encoding::Utf8 {
        return true;
    }
    for (i, r) in a.iter().enumerate() {
        if let View::Str(v) = r.bind(cx).view()
            && v.get(ns % lens[i]).enc() == Encoding::Utf8
        {
            return true;
        }
    }
    false
}

/// Format row `ns >= 1` under `&Cx` (no coercions or R callbacks run after
/// row 0), reusing `bufs`. Returns `(use_utf8, row_trunc_warns)`.
#[allow(clippy::too_many_arguments)]
fn format_row_n<'r>(
    cx: &Cx,
    call: Root<'r>,
    fmt_arg: Root<'_>,
    nfmt: usize,
    a: &[Root<'_>],
    lens: &[usize],
    used: &mut [bool],
    ns: usize,
    bufs: &mut RowBufs,
) -> Result<(bool, usize), Diag<'r>> {
    let nargs = a.len();
    let fv = fmt_arg.bind(cx).as_vector::<Char<'_>>().unwrap();
    let fc = fv.get(ns % nfmt);
    let use_utf8 = row_use_utf8(cx, fc, a, lens, ns);
    let format = tr_cow(cx, fc, use_utf8);
    let n = format.len();
    if n > MAXLINE {
        return Err(Diag::error(format!("'fmt' length exceeds maximal format length {MAXLINE}")));
    }
    bufs.output.clear();
    let mut cnt = 0usize;
    let mut cur = 0usize;
    let mut row_trunc_warns = 0usize;
    while cur < n {
        let cf = &format[cur..];
        if format[cur] != b'%' {
            let chunk = cf.iter().position(|&c| c == b'%').unwrap_or(cf.len());
            bufs.output.extend_from_slice(&cf[..chunk]);
            cur += chunk;
            continue;
        }
        if cur < n - 1 && format[cur + 1] == b'%' {
            bufs.output.push(b'%');
            cur += 2;
            continue;
        }
        let chunk = cf[1..].iter().position(|c| b"diosfeEgGxXaA".contains(c)).unwrap_or(cf.len() - 1) + 2;
        if cur + chunk > n {
            return Err(Diag::error(format!("unrecognised format specification '{}'", String::from_utf8_lossy(cf))));
        }
        bufs.fmt.clear();
        bufs.fmt.extend_from_slice(&cf[..chunk]);
        cur += chunk;
        // %n$ and %nn$
        let nthis = strip_arg_index(&mut bufs.fmt, 0, nargs)?;
        // `*`
        let mut star: Option<i32> = None;
        if let Some(sp) = bufs.fmt.iter().position(|&c| c == b'*') {
            let nstar = strip_arg_index(&mut bufs.fmt, sp, nargs)?;
            let k = match nstar {
                Some(k) => k,
                None => {
                    if cnt >= nargs {
                        return Err(Diag::error("too few arguments"));
                    }
                    cnt += 1;
                    cnt - 1
                }
            };
            if bufs.fmt[sp + 1..].contains(&b'*') {
                return Err(Diag::error("at most one asterisk '*' is supported in each conversion specification"));
            }
            used[k] = true;
            let t = a[k].bind(cx);
            let v = match t.as_vector::<i32>() {
                Some(v) if t.sexptype() == INTSXP && !v.is_empty() => v.get(ns % v.len()),
                _ => i32::MIN,
            };
            if v == i32::MIN {
                return Err(Diag::error("argument for '*' conversion specification must be a number"));
            }
            star = Some(v);
        }
        if bufs.fmt.last() == Some(&b'%') {
            // "%5%" and the like: copied as written (or with `*`)
            if star.is_some() {
                return Err(not_yet_diag(call, "sprintf() of a '%' conversion with a '*' width"));
            }
            bufs.output.extend_from_slice(&bufs.fmt);
            continue;
        }
        let k = match nthis {
            Some(k) => k,
            None => {
                if cnt >= nargs {
                    return Err(Diag::error("too few arguments"));
                }
                cnt += 1;
                cnt - 1
            }
        };
        used[k] = true;
        bufs.fmtp.clear();
        match star {
            Some(w) => expand_star_into(&bufs.fmt, w, &mut bufs.fmtp),
            None => bufs.fmtp.extend_from_slice(&bufs.fmt),
        }
        if bufs.fmtp.len() > MAXLINE {
            return Err(Diag::error(format!("'fmt' length exceeds maximal format length {MAXLINE}")));
        }
        let this = a[k].bind(cx);
        let thislen = lens[k];
        debug_assert_eq!(thislen, crate::coerce::xlength(this));
        if thislen == 0 {
            return Err(Diag::error("coercion has changed vector length to 0"));
        }
        format_value(
            cx,
            call,
            this,
            ns,
            thislen,
            &mut bufs.fmtp,
            use_utf8,
            &mut bufs.output,
            &mut bufs.bit,
            &mut row_trunc_warns,
        )?;
    }
    Ok((use_utf8, row_trunc_warns))
}

/// `.Internal(sprintf(fmt, ...))`.
// R: sprintf.c:do_sprintf
pub fn do_sprintf(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    if args.is_empty() || args.get(cx, 0).sexptype() != STRSXP {
        return Err(cx.error("'fmt' is not a character vector"));
    }
    let fmt_arg = args.root(0);
    let nfmt = fmt_arg.bind(cx).len().unwrap_or(0);
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
        let mut bufs = RowBufs::new();

        // Row 0 runs first under `&mut Cx` because minimal coercion (including
        // `as.double` / `as.character` R calls) may replace `a[k]` and adjust
        // `maxlen` before `allocVector(STRSXP, maxlen)` (sprintf.c:469).
        let (row0, row0_utf8) = {
            let (format, use_utf8) = {
                let fv = fmt_arg.bind(cx).as_vector::<Char<'_>>().unwrap();
                let fc = fv.get(0);
                let u = row_use_utf8(cx, fc, &a, &lens, 0);
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
                bufs.fmt.clear();
                bufs.fmt.extend_from_slice(&cf[..chunk]);
                cur += chunk;
                // %n$ and %nn$
                let nthis = match strip_arg_index(&mut bufs.fmt, 0, nargs) {
                    Ok(v) => v,
                    Err(d) => return Err(cx.raise(d)),
                };
                // `*`
                let mut star: Option<i32> = None;
                if let Some(sp) = bufs.fmt.iter().position(|&c| c == b'*') {
                    let nstar = match strip_arg_index(&mut bufs.fmt, sp, nargs) {
                        Ok(v) => v,
                        Err(d) => return Err(cx.raise(d)),
                    };
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
                    if bufs.fmt[sp + 1..].contains(&b'*') {
                        return Err(cx.error("at most one asterisk '*' is supported in each conversion specification"));
                    }
                    used[k] = true;
                    if a[k].bind(cx).sexptype() == REALSXP {
                        let r = coerced(cx, s, a[k], INTSXP)?;
                        a[k] = r;
                    }
                    let t = a[k].bind(cx);
                    let v = match t.as_vector::<i32>() {
                        Some(v) if t.sexptype() == INTSXP && !v.is_empty() => v.get(0),
                        _ => i32::MIN,
                    };
                    if v == i32::MIN {
                        return Err(cx.error("argument for '*' conversion specification must be a number"));
                    }
                    star = Some(v);
                }
                if bufs.fmt.last() == Some(&b'%') {
                    // "%5%" and the like: copied as written (or with `*`)
                    if star.is_some() {
                        return Err(not_yet(cx, Some(call), "sprintf() of a '%' conversion with a '*' width"));
                    }
                    output.extend_from_slice(&bufs.fmt);
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
                bufs.fmtp.clear();
                match star {
                    Some(w) => expand_star_into(&bufs.fmt, w, &mut bufs.fmtp),
                    None => bufs.fmtp.extend_from_slice(&bufs.fmt),
                }
                if bufs.fmtp.len() > MAXLINE {
                    return Err(cx.error(&format!("'fmt' length exceeds maximal format length {MAXLINE}")));
                }
                // minimal coercion, once, for the first row
                {
                    let c = bufs.fmtp.get(findspec(&bufs.fmtp)).copied().unwrap_or(0);
                    let t = a[k].bind(cx).sexptype();
                    match c {
                        b'd' | b'i' | b'o' | b'x' | b'X' if t == REALSXP => {
                            let exact = a[k]
                                .bind(cx)
                                .as_vector::<f64>()
                                .unwrap()
                                .as_slice()
                                .unwrap()
                                .iter()
                                .all(|&r| robigo_text::is_na(r) || (r.is_finite() && (r as i32) as f64 == r));
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
                let mut conv_warns = 0usize;
                let res = {
                    let this = a[k].bind(cx);
                    format_value(
                        cx,
                        call,
                        this,
                        0,
                        thislen,
                        &mut bufs.fmtp,
                        use_utf8,
                        &mut output,
                        &mut bufs.bit,
                        &mut conv_warns,
                    )
                };
                for _ in 0..conv_warns {
                    cx.warning(&trunc_warn_msg())?;
                }
                if let Err(d) = res {
                    return Err(cx.raise(d));
                }
            }
            (output, use_utf8)
        };

        // Rows 0..maxlen into `out` via `Cx::fill_strings` so collections run
        // between elements (E4).
        let mut trunc_warns = 0usize;
        cx.fill_strings(out, maxlen, |cx, fill| {
            let mut ns = fill.start();
            if ns == 0 {
                let enc = if row0_utf8 { Encoding::Utf8 } else { Encoding::Native };
                if fill.put(0, mk_char_checked(cx, &row0, enc))? {
                    return Ok(Ok(()));
                }
                ns = 1;
            }
            while ns < maxlen {
                match format_row_n(cx, call, fmt_arg, nfmt, &a, &lens, &mut used, ns, &mut bufs) {
                    Ok((use_utf8, row_warns)) => {
                        let enc = if use_utf8 { Encoding::Utf8 } else { Encoding::Native };
                        let stop = fill.put(ns, mk_char_checked(cx, &bufs.output, enc))?;
                        // Only count warnings after `fill.put` succeeds so a
                        // GC refusal retry on `ns` never double-counts.
                        trunc_warns += row_warns;
                        if stop {
                            return Ok(Ok(()));
                        }
                    }
                    Err(d) => {
                        let d = if trunc_warns > 0 { d.after_warnings(vec![trunc_warn_msg(); trunc_warns]) } else { d };
                        return Ok(Err(d));
                    }
                }
                ns += 1;
            }
            Ok(Ok(()))
        })?;
        for _ in 0..trunc_warns {
            cx.warning(&trunc_warn_msg())?;
        }

        let nunused = used.iter().filter(|u| !**u).count();
        if nunused > 0 {
            let msg = if nfmt == 1 {
                let f = {
                    let fc = fmt_arg.bind(cx).as_vector::<Char<'_>>().unwrap().get(0);
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
            return cx.warning(&msg);
        }
        Ok(())
    })
}

/// `TRANSLATE_CHAR(x, i)`: UTF-8 or native bytes of a CHARSXP, borrowing
/// directly when no translation is needed.
fn tr_cow<'a>(cx: &Cx, c: Char<'a>, utf8: bool) -> Cow<'a, [u8]> {
    let s = crate::coerce::text(c);
    let r = if utf8 { crate::text::to_utf8(s, cx.locale()) } else { crate::text::to_native(s, cx.locale()) };
    r.unwrap_or_else(|_| Cow::Borrowed(c.bytes()))
}

/// `TRANSLATE_CHAR(x, i)`: owned bytes of a CHARSXP (for outside `&Cx`).
fn tr(cx: &Cx, c: Char<'_>, utf8: bool) -> Vec<u8> {
    tr_cow(cx, c, utf8).into_owned()
}

/// `fmt` with its `*` replaced by the width `w`, appended into `out`.
fn expand_star_into(fmt: &[u8], w: i32, out: &mut Vec<u8>) {
    out.reserve(fmt.len() + 8);
    for &c in fmt {
        if c == b'*' {
            out.extend_from_slice(w.to_string().as_bytes());
        } else {
            out.push(c);
        }
    }
}

/// GNU R's "invalid format '%s'; %s" as a [`Diag`].
fn invalid_format_diag<'r>(fmt: &[u8], hint: &str) -> Diag<'r> {
    Diag::error(format!("invalid format '{}'; {hint}", String::from_utf8_lossy(fmt)))
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
            cx.lang2(f.into(), x.bind(cx))
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
