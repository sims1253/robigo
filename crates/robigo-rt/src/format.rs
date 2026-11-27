//! `.Internal(format(...))` and `.Internal(format.info(...))` (paste.c's
//! `do_format`, `do_formatinfo`) and `.Internal(formatC(...))` (util.c's
//! `do_formatC` with its `str_signif`): R's formatting of atomic vectors,
//! over robigo-text's ports of `formatReal`, `EncodeReal0` and friends and
//! of glibc's `printf`, with `PrintDefaults()`'s options
//! ([`crate::print::PrintData::init`]).

use crate::print::PrintData;
use robigo_heap::types::*;
use robigo_rtbuiltins::util::{as_logical, not_yet};
use robigo_rtcore::{Args, Char, Cx, Diag, Encoding, LARGE, Lgl, NeedGc, Out, RResult, Rcomplex, Ref, Root, View};
use robigo_text::{Str, num};
use std::borrow::Cow;

const NA: i32 = i32::MIN;

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

/// `asInteger(x)` with its coercion warnings.
fn as_integer(cx: &mut Cx, x: Root<'_>) -> RResult<i32> {
    robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, x)
}

/// `isVectorAtomic(x)`.
fn is_vector_atomic(x: Ref<'_>) -> bool {
    matches!(x.sexptype(), LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP)
}

/// Translate one non-NA element of a character vector as `do_format` does:
/// a "bytes" string with each byte outside `0x20..0x80` written `\xhh`,
/// another `translateChar`'d (returning its error if untranslatable).
/// Borrows directly from `c` when unchanged; writes into `scratch` otherwise.
// R: paste.c:do_format
fn format_string_elt<'a>(
    c: Char<'a>,
    loc: robigo_rtcore::Locale,
    scratch: &'a mut Vec<u8>,
) -> Result<(&'a [u8], Encoding), String> {
    if c.enc() == Encoding::Bytes {
        scratch.clear();
        let bytes = c.bytes();
        scratch.reserve(bytes.len());
        for &k in bytes {
            if (0x20..0x80).contains(&k) {
                scratch.push(k);
            } else {
                scratch.extend_from_slice(format!("\\x{k:02x}").as_bytes());
            }
        }
        return Ok((scratch.as_slice(), Encoding::Native));
    }
    match robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), loc) {
        // (translateChar returns CHAR(x) itself when nothing changes: the
        // element keeps its CHARSXP, and so its mark)
        Ok(Cow::Borrowed(b)) => Ok((b, c.enc())),
        Ok(Cow::Owned(b)) if b.as_slice() == c.bytes() => Ok((c.bytes(), c.enc())),
        Ok(Cow::Owned(b)) => {
            *scratch = b;
            Ok((scratch.as_slice(), Encoding::Native))
        }
        Err(e) => Err(e.to_string()),
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
    let res = out.root();
    let warnings = if n == 0 {
        cx.build(out, |cx| Ok(cx.alloc_vector::<Char<'_>>(0)?.finish().into()))?;
        0
    } else {
        fill_format_elements(cx, x, &pd, (trim, wd, adj, na != 0, nsmall), &dec, out)?
    };
    attach_format_attrs(cx, x, res)?;
    if warnings > 0
        && let Some(m) = {
            let pp = pd.params(cx);
            num::decimal_mark_warning(&dec, &pp)
        }
    {
        for _ in 0..warnings {
            cx.warning(m)?;
        }
    }
    Ok(())
}

/// Copy `dim` + `dimnames` (or `names`) from `x` onto the formatted result
/// `res`, matching `paste.c:do_format`.
fn attach_format_attrs(cx: &mut Cx, x: Root<'_>, res: Root<'_>) -> RResult<()> {
    let has = {
        let xv = x.bind(cx);
        robigo_rtbuiltins::attrib::get_value(xv, b"dim").is_some()
            || robigo_rtbuiltins::attrib::get_value(xv, b"names").is_some()
    };
    if !has {
        return Ok(());
    }
    cx.scope(|cx, s| {
        let al = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let xv = x.bind(cx);
                let mut a = robigo_rtbuiltins::attrib::Attrs::new();
                if let Some(d) = robigo_rtbuiltins::attrib::get_value(xv, b"dim") {
                    a.install(robigo_rtbuiltins::attrib::Attr { tag: cx.sym(robigo_rtcore::RSym::Dim), value: d });
                    if let Some(dn) = robigo_rtbuiltins::attrib::get_value(xv, b"dimnames") {
                        a.install(robigo_rtbuiltins::attrib::Attr {
                            tag: cx.sym(robigo_rtcore::RSym::DimNames),
                            value: dn,
                        });
                    }
                } else if let Some(nm) = robigo_rtbuiltins::attrib::get_value(xv, b"names") {
                    a.install(robigo_rtbuiltins::attrib::Attr { tag: cx.sym(robigo_rtcore::RSym::Names), value: nm });
                }
                a.pairlist(cx)
            })
        })?;
        if al.bind(cx).sexptype() != NILSXP {
            cx.set_attrib_list(res, al);
        }
        Ok(())
    })
}

/// The `n > 0` part of `do_format`, built element by element through
/// [`Cx::fill_strings`] so collections run between elements (E4). Returns the
/// number of `decimal_mark_warning`s to emit.
// R: paste.c:do_format
fn fill_format_elements(
    cx: &mut Cx,
    x: Root<'_>,
    pd: &PrintData,
    (trim, wd, adj, na, nsmall): (bool, i32, i32, bool, i32),
    dec: &[u8],
    out: Out<'_>,
) -> RResult<usize> {
    let n = x.bind(cx).len().unwrap();
    let ty = x.bind(cx).sexptype();
    let mut buf = Vec::new();
    match ty {
        LGLSXP => {
            let w = {
                let pp = pd.params(cx);
                let View::Lgl(v) = x.bind(cx).view() else { unreachable!() };
                if trim {
                    0
                } else {
                    let xs: Vec<i32> = v.as_slice().unwrap().iter().map(|l| l.0).collect();
                    num::format_logical(&xs, &pp)
                }
                .max(wd)
            };
            cx.fill_strings(out, n, |cx, fill| {
                let pp = pd.params(cx);
                let View::Lgl(v) = x.bind(cx).view() else { unreachable!() };
                for i in fill.start()..n {
                    buf.clear();
                    num::encode_logical(&mut buf, v.get(i).0, w, &pp);
                    if fill.put(i, mk_char_checked(cx, &buf, Encoding::Native))? {
                        return Ok(Ok(()));
                    }
                }
                Ok(Ok(()))
            })?;
            Ok(0)
        }
        INTSXP => {
            let w = {
                let pp = pd.params(cx);
                let View::Int(v) = x.bind(cx).view() else { unreachable!() };
                if trim { 0 } else { num::format_integer(v.as_slice().unwrap(), &pp) }.max(wd)
            };
            cx.fill_strings(out, n, |cx, fill| {
                let pp = pd.params(cx);
                let View::Int(v) = x.bind(cx).view() else { unreachable!() };
                for i in fill.start()..n {
                    buf.clear();
                    num::encode_integer(&mut buf, v.get(i), w, &pp);
                    if fill.put(i, mk_char_checked(cx, &buf, Encoding::Native))? {
                        return Ok(Ok(()));
                    }
                }
                Ok(Ok(()))
            })?;
            Ok(0)
        }
        REALSXP => {
            let f = {
                let pp = pd.params(cx);
                let View::Real(v) = x.bind(cx).view() else { unreachable!() };
                let mut f = num::format_real(v.as_slice().unwrap(), nsmall, &pp);
                if trim {
                    f.w = 0;
                }
                f.w = f.w.max(wd);
                f
            };
            cx.fill_strings(out, n, |cx, fill| {
                let pp = pd.params(cx);
                let View::Real(v) = x.bind(cx).view() else { unreachable!() };
                for i in fill.start()..n {
                    buf.clear();
                    num::encode_real0(&mut buf, v.get(i), f, dec, &pp);
                    if fill.put(i, mk_char_checked(cx, &buf, Encoding::Native))? {
                        return Ok(Ok(()));
                    }
                }
                Ok(Ok(()))
            })?;
            Ok(n)
        }
        CPLXSXP => {
            let (fr, fi, warnings) = {
                let pp = pd.params(cx);
                let View::Cplx(v) = x.bind(cx).view() else { unreachable!() };
                let xs: Vec<(f64, f64)> = v.as_slice().unwrap().iter().map(|c: &Rcomplex| (c.r, c.i)).collect();
                let (mut fr, mut fi) = num::format_complex(&xs, nsmall, &pp);
                if trim {
                    fr.w = 0;
                    fi.w = 0;
                }
                fr.w = fr.w.max(wd);
                fi.w = fi.w.max(wd);
                // (EncodeComplex: EncodeReal0 of each part, NA aside)
                let warnings =
                    xs.iter().filter(|&&(r, i)| !(robigo_text::is_na(r) || robigo_text::is_na(i))).count() * 2;
                (fr, fi, warnings)
            };
            cx.fill_strings(out, n, |cx, fill| {
                let pp = pd.params(cx);
                let View::Cplx(v) = x.bind(cx).view() else { unreachable!() };
                for i in fill.start()..n {
                    let c = v.get(i);
                    buf.clear();
                    num::encode_complex(&mut buf, (c.r, c.i), fr, fi, dec, &pp);
                    if fill.put(i, mk_char_checked(cx, &buf, Encoding::Native))? {
                        return Ok(Ok(()));
                    }
                }
                Ok(Ok(()))
            })?;
            Ok(warnings)
        }
        STRSXP => {
            // (this has to be different from formatString/EncodeString as we
            // don't actually want to encode here)
            let mut tr_buf = Vec::new();
            let (w, err) = {
                let pp = pd.params(cx);
                let loc = cx.locale();
                let v = x.bind(cx).as_vector::<Char<'_>>().unwrap();
                let na_s = cx.na_string();
                let mut w = if adj != 3 { wd } else { 0 };
                let mut err: Option<String> = None;
                for i in 0..n {
                    let c = v.get(i);
                    if c == na_s {
                        if adj != 3 && na {
                            w = w.max(pp.na_width);
                        }
                        continue;
                    }
                    match format_string_elt(c, loc, &mut tr_buf) {
                        Ok((bytes, enc)) => {
                            if adj != 3 {
                                w = w.max(robigo_text::string::rstrlen(text_slice(bytes, enc), 0, pp.locale));
                            }
                        }
                        Err(e) => {
                            err = Some(e);
                            break;
                        }
                    }
                }
                (w, err)
            };
            if let Some(e) = err {
                return Err(cx.error(&e));
            }
            cx.fill_strings(out, n, |cx, fill| {
                let pp = pd.params(cx);
                let loc = cx.locale();
                let v = x.bind(cx).as_vector::<Char<'_>>().unwrap();
                let na_s = cx.na_string();
                for i in fill.start()..n {
                    let c = v.get(i);
                    if !na && c == na_s {
                        if fill.put(i, Ok(na_s))? {
                            return Ok(Ok(()));
                        }
                        continue;
                    }
                    let (s0, il) = if c == na_s {
                        (pd.na_string.0.as_slice(), pp.na_width)
                    } else {
                        let (bytes, enc) = format_string_elt(c, loc, &mut tr_buf).expect("checked in width pass");
                        let il = robigo_text::string::rstrlen(text_slice(bytes, enc), 0, pp.locale);
                        (bytes, il)
                    };
                    let mut b = w - il;
                    buf.clear();
                    buf.reserve(s0.len() + b.max(0) as usize);
                    if b > 0 && adj != 0 {
                        let b0 = if adj == 2 { b / 2 } else { b };
                        buf.extend(std::iter::repeat_n(b' ', b0 as usize));
                        b -= b0;
                    }
                    buf.extend_from_slice(s0);
                    if b > 0 && adj != 1 {
                        buf.extend(std::iter::repeat_n(b' ', b as usize));
                    }
                    // mkChar(buff): native
                    if fill.put(i, mk_char_checked(cx, &buf, Encoding::Native))? {
                        return Ok(Ok(()));
                    }
                }
                Ok(Ok(()))
            })?;
            Ok(0)
        }
        _ => Err(cx.error("Impossible mode ( x )")),
    }
}

/// robigo-text's view of a translated string slice and its encoding mark.
fn text_slice(bytes: &[u8], enc: Encoding) -> Str<'_> {
    Str::new(bytes, robigo_rtbuiltins::coerce::text_enc(enc))
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
    cx.scope(|cx, s| {
        let lens =
            s.capture(cx, |cx, o| robigo_rtbuiltins::coerce::vector::coerce_vector(cx, args.root(6), INTSXP, o))?;
        cx.materialize(lens)?;
        cx.materialize(x)?;
        let n = x.bind(cx).len().unwrap();
        let dig = digits.wrapping_abs();
        let do_fg = fmt == b"fg";
        if width == 0 {
            return Err(cx.error("width cannot be zero"));
        }
        let mut buf = Vec::new();
        if fmt == b"d" {
            let mut form = b"%".to_vec();
            form.extend_from_slice(&flag);
            form.extend_from_slice(b"*d");
            if ty != b"integer" {
                return Err(cx.error("'type' must be \"integer\" for  \"d\"-format"));
            }
            if x.bind(cx).sexptype() != INTSXP {
                return Err(not_yet(cx, Some(call), "formatC() of this type"));
            }
            let spec = if n > 0 {
                match parse_star(&form, width, None) {
                    Ok(s) => Some(s),
                    Err(Signif::NotYet(m)) => return Err(not_yet(cx, Some(call), m)),
                }
            } else {
                None
            };
            return cx.fill_strings(out, n, |cx, fill| {
                let Some(spec) = spec.as_ref() else {
                    return Ok(Ok(()));
                };
                let View::Int(v) = x.bind(cx).view() else { unreachable!() };
                let lv = lens.bind(cx).as_vector::<i32>().unwrap();
                for i in fill.start()..n {
                    buf.clear();
                    spec.int(&mut buf, v.get(i));
                    cut_buf(&mut buf, lv.get(i));
                    if fill.put(i, mk_char_checked(cx, &buf, Encoding::Native))? {
                        return Ok(Ok(()));
                    }
                }
                Ok(Ok(()))
            });
        }
        // --- floating point ---
        let mut form = b"%".to_vec();
        form.extend_from_slice(&flag);
        form.extend_from_slice(b"*.*");
        if do_fg {
            form.push(b'g');
        } else {
            form.extend_from_slice(&fmt);
        }
        if ty != b"double" {
            return Err(cx.error("'type' must be \"real\" for this format"));
        }
        if x.bind(cx).sexptype() != REALSXP {
            return Err(not_yet(cx, Some(call), "formatC() of this type"));
        }
        let mut cached_spec: Option<robigo_text::printf::Spec> = None;
        cx.fill_strings(out, n, |cx, fill| {
            let View::Real(v) = x.bind(cx).view() else { unreachable!() };
            let lv = lens.bind(cx).as_vector::<i32>().unwrap();
            for i in fill.start()..n {
                let mut xx = v.get(i);
                if !xx.is_finite() {
                    // (formatC's R code replaced every non-finite number)
                    return Ok(Err(Diag::error_call(
                        Some(call),
                        "robigo: formatC() of a non-finite number is not implemented yet",
                    )));
                }
                buf.clear();
                if do_fg {
                    // do smart "f"
                    if xx == 0.0 {
                        buf.extend_from_slice(b"0");
                        cut_buf(&mut buf, lv.get(i));
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
                            ff.extend_from_slice(&flag);
                            ff.extend_from_slice(format!(".{}f", dig - 1 - iex).as_bytes());
                            let filtered: Vec<u8> = ff.into_iter().filter(|&c| c != b'\'' && c != b'I').collect();
                            let Some(s) = robigo_text::printf::Spec::parse(&filtered) else {
                                return Ok(Err(Diag::error_call(
                                    Some(call),
                                    "robigo: formatC() with this 'flag' is not implemented yet",
                                )));
                            };
                            s.double(&mut buf, xx);
                            cut_buf(&mut buf, lv.get(i));
                            // Remove trailing "0"s __ IFF flag has no '#'
                            if digits >= 0 {
                                while buf.last() == Some(&b'0') {
                                    buf.pop();
                                }
                            }
                        } else {
                            // iex >= -4: NOT "e-"; if iex >= dig, would have "e+"
                            let prec = if iex >= dig { iex + 1 } else { dig };
                            match parse_star(&form, width, Some(prec)) {
                                Ok(s) => s.double(&mut buf, xx),
                                Err(Signif::NotYet(m)) => {
                                    return Ok(Err(Diag::error_call(
                                        Some(call),
                                        format!("robigo: {m} is not implemented yet"),
                                    )));
                                }
                            }
                            cut_buf(&mut buf, lv.get(i));
                        }
                    }
                } else {
                    let s = match &cached_spec {
                        Some(s) => s,
                        None => match parse_star(&form, width, Some(dig)) {
                            Ok(s) => cached_spec.insert(s),
                            Err(Signif::NotYet(m)) => {
                                return Ok(Err(Diag::error_call(
                                    Some(call),
                                    format!("robigo: {m} is not implemented yet"),
                                )));
                            }
                        },
                    };
                    s.double(&mut buf, xx);
                    cut_buf(&mut buf, lv.get(i));
                }
                if fill.put(i, mk_char_checked(cx, &buf, Encoding::Native))? {
                    return Ok(Ok(()));
                }
            }
            Ok(Ok(()))
        })
    })
}

/// Truncate `buf` to `i.strlen + 2` bytes as `do_formatC`'s `snprintf` buffer does.
#[inline]
fn cut_buf(buf: &mut Vec<u8>, len: i32) {
    let cap = (len as i64 + 2).max(0) as usize;
    buf.truncate(cap);
}

/// Why `parse_star` stopped.
enum Signif {
    NotYet(&'static str),
}

/// Parse one conversion spec `form` with its `*`s given (`width`, optional
/// `prec`) into `robigo_text::printf::Spec`.
fn parse_star(form: &[u8], width: i32, prec: Option<i32>) -> Result<robigo_text::printf::Spec, Signif> {
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
    robigo_text::printf::Spec::parse(&spec).ok_or(Signif::NotYet("formatC() with this 'flag'"))
}
