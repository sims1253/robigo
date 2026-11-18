//! `.Internal(format(...))` and `.Internal(format.info(...))` (paste.c's
//! `do_format`, `do_formatinfo`) and `.Internal(formatC(...))` (util.c's
//! `do_formatC` with its `str_signif`): R's formatting of atomic vectors,
//! over robigo-text's ports of `formatReal`, `EncodeReal0` and friends and
//! of glibc's `printf`, with `PrintDefaults()`'s options
//! ([`crate::print::PrintData::init`]).

use crate::print::PrintData;
use robigo_heap::types::*;
use robigo_rtbuiltins::util::{as_logical, not_yet};
use robigo_rtcore::{Args, Char, Cx, Encoding, Lgl, Out, RResult, Rcomplex, Ref, Root, View};
use robigo_text::{Str, num};

const NA: i32 = i32::MIN;

/// `asInteger(x)` with its coercion warnings.
fn as_integer(cx: &mut Cx, x: Root<'_>) -> RResult<i32> {
    robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, x)
}

/// `isVectorAtomic(x)`.
fn is_vector_atomic(x: Ref<'_>) -> bool {
    matches!(x.sexptype(), LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP)
}

/// The element `i` of a character vector translated as `do_format` does:
/// a "bytes" string with each byte outside 0x20..0x7f written `\xhh`,
/// another `translateChar`'d (its error for one that cannot be); None for
/// NA.
// R: paste.c:do_format
fn format_string_elt(cx: &mut Cx, x: Root<'_>, i: usize) -> RResult<Option<(Vec<u8>, Encoding)>> {
    let (na, enc, bytes) = {
        let c = x.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
        (c == cx.na_string(), c.enc(), c.bytes().to_vec())
    };
    if na {
        return Ok(None);
    }
    if enc == Encoding::Bytes {
        let mut out = Vec::with_capacity(bytes.len());
        for &k in &bytes {
            if (0x20..0x80).contains(&k) {
                out.push(k);
            } else {
                out.extend_from_slice(format!("\\x{k:02x}").as_bytes());
            }
        }
        return Ok(Some((out, Encoding::Native)));
    }
    let tr = {
        let c = x.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
        robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), cx.locale()).map(|b| b.into_owned())
    };
    match tr {
        // (translateChar returns CHAR(x) itself when nothing changes: the
        // element keeps its CHARSXP, and so its mark)
        Ok(b) if b == bytes => Ok(Some((b, enc))),
        Ok(b) => Ok(Some((b, Encoding::Native))),
        Err(e) => Err(cx.error(&e.to_string())),
    }
}

/// `format.default(x, trim, digits, nsmall, width, justify, na.encode,
/// scientific, decimal.mark)`'s `.Internal(format(...))`.
// R: paste.c:do_format
pub fn do_format(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "format", 9, true)?;
    let pd = PrintData::init(cx, None)?;
    let mut digits = pd.digits;
    let mut scipen = pd.scipen;
    let x = args.root(0);
    match x.bind(cx).sexptype() {
        ENVSXP => {
            let s = crate::print::encode_environment(cx, x)?;
            return cx.build(out, |cx| cx.mk_string(&s, Encoding::Native));
        }
        EXTPTRSXP => {
            // EncodeExtptr: R_ExternalPtrAddr(x)
            let p = match x.bind(cx).view() {
                View::ExtPtr(e) => e.addr() as usize,
                _ => unreachable!("robigo: an EXTPTRSXP's view"),
            };
            let s = format!("<pointer: {}>", crate::print::c_pointer(p));
            return cx.build(out, |cx| cx.mk_string(s.as_bytes(), Encoding::Native));
        }
        _ if !is_vector_atomic(x.bind(cx)) => return Err(cx.error("first argument must be atomic or environment")),
        _ => {}
    }
    let trim = as_logical(cx, args.get(cx, 1));
    if trim == NA {
        return Err(cx.error("invalid 'trim' argument"));
    }
    if args.get(cx, 2).sexptype() != NILSXP {
        let d = as_integer(cx, args.root(2))?;
        if d == NA || !(1..=22).contains(&d) {
            return Err(cx.error(&format!("invalid value {d} for 'digits' argument")));
        }
        digits = d;
    }
    let nsmall = as_integer(cx, args.root(3))?;
    if nsmall == NA || !(0..=20).contains(&nsmall) {
        return Err(cx.error("invalid 'nsmall' argument"));
    }
    let wd = if args.get(cx, 4).sexptype() == NILSXP { 0 } else { as_integer(cx, args.root(4))? };
    if wd == NA {
        return Err(cx.error("invalid 'width' argument"));
    }
    let adj = as_integer(cx, args.root(5))?;
    if adj == NA || !(0..=3).contains(&adj) {
        return Err(cx.error("invalid 'justify' argument"));
    }
    let na = as_logical(cx, args.get(cx, 6));
    if na == NA {
        return Err(cx.error("invalid 'na.encode' argument"));
    }
    let sci_arg = args.get(cx, 7);
    if sci_arg.len() != Some(1) {
        return Err(cx.error("invalid 'scientific' argument"));
    }
    let sci = match sci_arg.view() {
        View::Lgl(v) => match v.get(0).0 {
            NA => NA,
            t if t > 0 => -99,
            _ => 310,
        },
        _ if matches!(sci_arg.sexptype(), INTSXP | REALSXP)
            && !robigo_rtbuiltins::attrib::inherits(sci_arg, b"factor") =>
        {
            as_integer(cx, args.root(7))?
        }
        _ => return Err(cx.error("invalid 'scientific' argument")),
    };
    if sci != NA {
        scipen = sci;
    }
    // copy/paste from "OutDec" part of ./options.c
    let dm = args.get(cx, 8);
    if dm.sexptype() != STRSXP || dm.len() != Some(1) {
        return Err(cx.error("invalid 'decimal.mark' argument"));
    }
    let dec: Vec<u8> = {
        let c = dm.as_vector::<Char<'_>>().unwrap().get(0);
        if c == cx.na_string() {
            // OutDec (options(OutDec = ) is not settable yet: ".")
            b".".to_vec()
        } else {
            c.bytes().iter().copied().take(10).collect()
        }
    };
    if x.bind(cx).is_package_altrep() {
        return Err(not_yet(cx, Some(call), "format() of a package ALTREP vector"));
    }
    cx.materialize(x)?;
    let mut pd = pd;
    pd.digits = digits;
    pd.scipen = scipen;
    let n = x.bind(cx).len().unwrap();
    let trim = trim != 0;
    // the strings, built before anything allocates
    let ys: Vec<Option<(Vec<u8>, Encoding)>> =
        if n == 0 { Vec::new() } else { format_elements(cx, x, &pd, (trim, wd, adj, na != 0, nsmall), &dec)? };
    cx.build(out, |cx| {
        let xv = x.bind(cx);
        let mut y = cx.alloc_vector::<Char<'_>>(n)?;
        for (i, s) in ys.iter().enumerate() {
            y.set(
                i,
                match s {
                    None => cx.na_string(),
                    Some((b, e)) => cx.mk_char(b, *e)?,
                },
            );
        }
        let mut a = robigo_rtbuiltins::attrib::Attrs::new();
        if let Some(d) = robigo_rtbuiltins::attrib::get_value(xv, b"dim") {
            a.install(robigo_rtbuiltins::attrib::Attr { tag: cx.sym(robigo_rtcore::RSym::Dim), value: d });
            if let Some(dn) = robigo_rtbuiltins::attrib::get_value(xv, b"dimnames") {
                a.install(robigo_rtbuiltins::attrib::Attr { tag: cx.sym(robigo_rtcore::RSym::DimNames), value: dn });
            }
        } else if let Some(nm) = robigo_rtbuiltins::attrib::get_value(xv, b"names") {
            a.install(robigo_rtbuiltins::attrib::Attr { tag: cx.sym(robigo_rtcore::RSym::Names), value: nm });
        }
        a.apply(cx, &mut y)?;
        Ok(y.finish().into())
    })
}

/// The `n > 0` part of `do_format`: each element's text (None: NA kept).
// R: paste.c:do_format
fn format_elements(
    cx: &mut Cx,
    x: Root<'_>,
    pd: &PrintData,
    (trim, wd, adj, na, nsmall): (bool, i32, i32, bool, i32),
    dec: &[u8],
) -> RResult<Vec<Option<(Vec<u8>, Encoding)>>> {
    let n = x.bind(cx).len().unwrap();
    let mut ys = Vec::with_capacity(n);
    let mut warnings = 0usize;
    let warning = {
        let pp = pd.params(cx);
        num::decimal_mark_warning(dec, &pp)
    };
    {
        let pp = pd.params(cx);
        let xv = x.bind(cx);
        let text = |out: Vec<u8>| Some((out, Encoding::Native));
        match xv.view() {
            View::Lgl(v) => {
                let xs: Vec<i32> = v.as_slice().unwrap().iter().map(|l| l.0).collect();
                let w = if trim { 0 } else { num::format_logical(&xs, &pp) }.max(wd);
                for &l in &xs {
                    let mut o = Vec::new();
                    num::encode_logical(&mut o, l, w, &pp);
                    ys.push(text(o));
                }
            }
            View::Int(v) => {
                let xs = v.as_slice().unwrap();
                let w = if trim { 0 } else { num::format_integer(xs, &pp) }.max(wd);
                for &k in xs {
                    let mut o = Vec::new();
                    num::encode_integer(&mut o, k, w, &pp);
                    ys.push(text(o));
                }
            }
            View::Real(v) => {
                let xs = v.as_slice().unwrap();
                let mut f = num::format_real(xs, nsmall, &pp);
                if trim {
                    f.w = 0;
                }
                f.w = f.w.max(wd);
                for &r in xs {
                    let mut o = Vec::new();
                    num::encode_real0(&mut o, r, f, dec, &pp);
                    warnings += 1;
                    ys.push(text(o));
                }
            }
            View::Cplx(v) => {
                let xs: Vec<(f64, f64)> = v.as_slice().unwrap().iter().map(|c: &Rcomplex| (c.r, c.i)).collect();
                let (mut fr, mut fi) = num::format_complex(&xs, nsmall, &pp);
                if trim {
                    fr.w = 0;
                    fi.w = 0;
                }
                fr.w = fr.w.max(wd);
                fi.w = fi.w.max(wd);
                for &c in &xs {
                    let mut o = Vec::new();
                    num::encode_complex(&mut o, c, fr, fi, dec, &pp);
                    // (EncodeComplex: EncodeReal0 of each part, NA aside)
                    if !(robigo_text::is_na(c.0) || robigo_text::is_na(c.1)) {
                        warnings += 2;
                    }
                    ys.push(text(o));
                }
            }
            View::Str(_) => {}
            _ => return Err(cx.error("Impossible mode ( x )")),
        }
    }
    if x.bind(cx).sexptype() == STRSXP {
        // (this has to be different from formatString/EncodeString as we
        // don't actually want to encode here)
        let mut xx = Vec::with_capacity(n);
        for i in 0..n {
            xx.push(format_string_elt(cx, x, i)?);
        }
        let pp = pd.params(cx);
        let loc = pp.locale;
        let width = |s: &(Vec<u8>, Encoding)| robigo_text::string::rstrlen(text_of(s), 0, loc);
        let mut w = wd;
        if adj != 3 {
            for s in &xx {
                match s {
                    Some(s) => w = w.max(width(s)),
                    None if na => w = w.max(pp.na_width),
                    None => {}
                }
            }
        } else {
            w = 0;
        }
        for s in &xx {
            if !na && s.is_none() {
                ys.push(None);
                continue;
            }
            let (s0, il) = match s {
                Some(s) => (s.0.clone(), width(s)),
                None => (pd.na_string.0.clone(), pp.na_width),
            };
            let mut b = w - il;
            let mut q = Vec::with_capacity(s0.len() + b.max(0) as usize);
            if b > 0 && adj != 0 {
                let b0 = if adj == 2 { b / 2 } else { b };
                q.extend(std::iter::repeat_n(b' ', b0 as usize));
                b -= b0;
            }
            q.extend_from_slice(&s0);
            if b > 0 && adj != 1 {
                q.extend(std::iter::repeat_n(b' ', b as usize));
            }
            // mkChar(buff): native
            ys.push(Some((q, Encoding::Native)));
        }
    }
    if let Some(m) = warning {
        for _ in 0..warnings {
            cx.warning(m)?;
        }
    }
    Ok(ys)
}

/// robigo-text's view of a translated string.
fn text_of(s: &(Vec<u8>, Encoding)) -> Str<'_> {
    robigo_rtbuiltins::coerce::text_held(s)
}

/// `.Internal(format.info(x, digits, nsmall))`: the width (and for doubles
/// the digits and exponent, for complex both parts' three) `format` uses.
// R: paste.c:do_formatinfo
pub fn do_formatinfo(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "format.info", 3, true)?;
    let x = args.root(0);
    let mut pd = PrintData::init(cx, None)?;
    if args.get(cx, 1).sexptype() != NILSXP {
        let d = as_integer(cx, args.root(1))?;
        if d == NA || !(1..=22).contains(&d) {
            return Err(cx.error("invalid 'digits' argument"));
        }
        pd.digits = d;
    }
    let nsmall = as_integer(cx, args.root(2))?;
    if nsmall == NA || !(0..=20).contains(&nsmall) {
        return Err(cx.error("invalid 'nsmall' argument"));
    }
    if x.bind(cx).is_package_altrep() {
        return Err(not_yet(cx, Some(call), "format.info() of a package ALTREP vector"));
    }
    cx.materialize(x)?;
    let vals: Vec<i32> = {
        let pp = pd.params(cx);
        let xv = x.bind(cx);
        match xv.view() {
            View::Raw(_) => vec![2],
            View::Lgl(v) => {
                let xs: Vec<i32> = v.as_slice().unwrap().iter().map(|l: &Lgl| l.0).collect();
                vec![num::format_logical(&xs, &pp)]
            }
            View::Int(v) => vec![num::format_integer(v.as_slice().unwrap(), &pp)],
            View::Real(v) => {
                let f = num::format_real(v.as_slice().unwrap(), nsmall, &pp);
                vec![f.w, f.d, f.e]
            }
            View::Cplx(v) => {
                let xs: Vec<(f64, f64)> = v.as_slice().unwrap().iter().map(|c| (c.r, c.i)).collect();
                let (fr, fi) = num::format_complex(&xs, nsmall, &pp);
                vec![fr.w, fr.d, fr.e, fi.w, fi.d, fi.e]
            }
            View::Str(v) => {
                let mut w = 0;
                for i in 0..v.len() {
                    let c = v.get(i);
                    if c != cx.na_string() {
                        w = w.max(robigo_text::string::rstrlen(robigo_rtbuiltins::coerce::text(c), 0, pp.locale));
                    }
                }
                vec![w]
            }
            _ => Vec::new(),
        }
    };
    if vals.is_empty() {
        return Err(cx.error("atomic vector arguments only"));
    }
    cx.build(out, |cx| Ok(cx.alloc_copy(&vals)?.finish().into()))
}

/// `.Internal(formatC(x, mode, width, digits, format, flag, i.strlen))`:
/// each number through C's `printf` as `str_signif` builds the format
/// (`%<flag>*d`, `%<flag>*.*<format>`, and "fg"'s significant digits), each
/// result cut to `i.strlen + 2` bytes as `snprintf` into its buffer cuts it.
/// (formatC's R code replaces non-finite numbers before this.)
// R: util.c:do_formatC, str_signif
pub fn do_formatc(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "formatC", 7, true)?;
    let x = args.root(0);
    if !robigo_rtbuiltins::attrib::is_vector(x.bind(cx)) {
        return Err(cx.error("'x' must be a vector"));
    }
    if (0..7).any(|i| args.get(cx, i).is_package_altrep()) {
        return Err(not_yet(cx, Some(call), "formatC() of a package ALTREP vector"));
    }
    let first = |cx: &Cx, i: usize| -> Vec<u8> {
        args.get(cx, i).as_vector::<Char<'_>>().map(|v| v.get(0).bytes().to_vec()).unwrap_or_default()
    };
    let ty = first(cx, 1);
    let width = as_integer(cx, args.root(2))?;
    let digits = as_integer(cx, args.root(3))?;
    let fmt = first(cx, 4);
    let flag = first(cx, 5);
    let lens = cx.scope(|cx, s| {
        let l = s.capture(cx, |cx, o| robigo_rtbuiltins::coerce::vector::coerce_vector(cx, args.root(6), INTSXP, o))?;
        cx.materialize(l)?;
        Ok(l.bind(cx).as_vector::<i32>().unwrap().as_slice().unwrap().to_vec())
    })?;
    cx.materialize(x)?;
    let n = x.bind(cx).len().unwrap();
    let res = {
        let xv = x.bind(cx);
        str_signif(xv, n, &ty, width, digits, &fmt, &flag, &lens)
    };
    let res = match res {
        Ok(r) => r,
        Err(Signif::Error(m)) => return Err(cx.error(m)),
        Err(Signif::NotYet(m)) => return Err(not_yet(cx, Some(call), m)),
    };
    cx.build(out, |cx| {
        let mut ans = cx.alloc_vector::<Char<'_>>(n)?;
        for (i, r) in res.iter().enumerate() {
            ans.set(i, cx.mk_char(r, Encoding::Native)?);
        }
        Ok(ans.finish().into())
    })
}

/// Why `str_signif` stopped.
enum Signif {
    Error(&'static str),
    NotYet(&'static str),
}

/// `printf(form, width, [prec,] x)` for one conversion spec `form` with
/// its `*`s given: robigo-text's glibc `printf`.
fn printf_star(form: &[u8], width: i32, prec: Option<i32>, out: &mut Vec<u8>, val: Val) -> Result<(), Signif> {
    // flags, then '*' (width), then ".*" (precision), then the conversion
    let body = &form[1..];
    let star = body.iter().position(|&c| c == b'*').unwrap();
    // glibc's ' (grouping) and I (locale digits) do nothing in robigo-rt's
    // locales (C and C.UTF-8)
    let mut flags: Vec<u8> = body[..star].iter().copied().filter(|&c| c != b'\'' && c != b'I').collect();
    // a negative width is a '-' flag and its absolute value
    if width < 0 {
        flags.push(b'-');
    }
    let rest = &body[star + 1..];
    let mut spec = b"%".to_vec();
    spec.extend_from_slice(&flags);
    spec.extend_from_slice(width.unsigned_abs().to_string().as_bytes());
    let conv = match prec {
        Some(p) => {
            // a negative precision is taken as if it were omitted
            if p >= 0 {
                spec.extend_from_slice(format!(".{p}").as_bytes());
            }
            &rest[2..]
        }
        None => rest,
    };
    spec.extend_from_slice(conv);
    let s = robigo_text::printf::Spec::parse(&spec).ok_or(Signif::NotYet("formatC() with this 'flag'"))?;
    match val {
        Val::Int(i) => s.int(out, i),
        Val::Dbl(d) => s.double(out, d),
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Val {
    Int(i32),
    Dbl(f64),
}

/// `str_signif(x, n, type, width, digits, format, flag, result)`, each
/// result cut to its buffer (`i.strlen + 2` bytes).
// R: util.c:str_signif, str_signif_sexp
#[allow(clippy::too_many_arguments)]
fn str_signif(
    x: Ref<'_>,
    n: usize,
    ty: &[u8],
    width: i32,
    digits: i32,
    format: &[u8],
    flag: &[u8],
    lens: &[i32],
) -> Result<Vec<Vec<u8>>, Signif> {
    let dig = digits.wrapping_abs();
    let do_fg = format == b"fg";
    if width == 0 {
        return Err(Signif::Error("width cannot be zero"));
    }
    let cut = |mut r: Vec<u8>, i: usize| {
        let cap = (lens[i] as i64 + 2).max(0) as usize;
        r.truncate(cap);
        r
    };
    let mut res = Vec::with_capacity(n);
    if format == b"d" {
        let mut form = b"%".to_vec();
        form.extend_from_slice(flag);
        form.extend_from_slice(b"*d");
        if ty != b"integer" {
            return Err(Signif::Error("'type' must be \"integer\" for  \"d\"-format"));
        }
        let View::Int(v) = x.view() else { return Err(Signif::NotYet("formatC() of this type")) };
        for i in 0..n {
            let mut o = Vec::new();
            printf_star(&form, width, None, &mut o, Val::Int(v.get(i)))?;
            res.push(cut(o, i));
        }
        return Ok(res);
    }
    // --- floating point ---
    let mut form = b"%".to_vec();
    form.extend_from_slice(flag);
    form.extend_from_slice(b"*.*");
    if do_fg {
        form.push(b'g');
    } else {
        form.extend_from_slice(format);
    }
    if ty != b"double" {
        return Err(Signif::Error("'type' must be \"real\" for this format"));
    }
    let View::Real(v) = x.view() else { return Err(Signif::NotYet("formatC() of this type")) };
    for i in 0..n {
        let mut xx = v.get(i);
        if !xx.is_finite() {
            // (formatC's R code replaced every non-finite number)
            return Err(Signif::NotYet("formatC() of a non-finite number"));
        }
        let mut o = Vec::new();
        if do_fg {
            // do smart "f"
            if xx == 0.0 {
                o.extend_from_slice(b"0");
            } else {
                let xxx = xx.abs();
                let mut iex = (xxx.log10() + 1e-12).floor() as i32;
                let x10 = robigo_numerics::fround::fround(xxx / 10f64.powi(iex) + 1e-12, (dig - 1) as f64);
                if iex > 0 && x10 >= 10.0 {
                    xx = x10 * 10f64.powi(iex);
                    iex += 1;
                }
                if iex == -4 && xx.abs() < 1e-4 {
                    // VERY rare case
                    iex = -5;
                }
                if iex < -4 {
                    // "g" would result in 'e-' representation: f0 is
                    // "%<flag>.*f" (no width)
                    let mut ff = b"%".to_vec();
                    ff.extend_from_slice(flag);
                    ff.extend_from_slice(format!(".{}f", dig - 1 - iex).as_bytes());
                    let s = robigo_text::printf::Spec::parse(
                        &ff.iter().copied().filter(|&c| c != b'\'' && c != b'I').collect::<Vec<u8>>(),
                    )
                    .ok_or(Signif::NotYet("formatC() with this 'flag'"))?;
                    s.double(&mut o, xx);
                    let mut o = cut(o, i);
                    // Remove trailing "0"s __ IFF flag has no '#'
                    if digits >= 0 {
                        while o.last() == Some(&b'0') {
                            o.pop();
                        }
                    }
                    res.push(o);
                    continue;
                }
                // iex >= -4: NOT "e-"; if iex >= dig, would have "e+"
                printf_star(&form, width, Some(if iex >= dig { iex + 1 } else { dig }), &mut o, Val::Dbl(xx))?;
            }
        } else {
            printf_star(&form, width, Some(dig), &mut o, Val::Dbl(xx))?;
        }
        res.push(cut(o, i));
    }
    Ok(res)
}
