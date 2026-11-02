//! `sprintf()` (GNU R's sprintf.c), plus a port of the C library's printf
//! conversions that R relies on (also used by `formatC()`).

use super::internal;
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::value::*;

pub static DEFS: &[Builtin] = &[internal("sprintf", do_sprintf, -1)];

// ----------------------------------------------------------------------
// C printf conversions (glibc behaviour)

/// A parsed `%[flags][width][.precision]conversion` specification.
#[derive(Clone, Debug, Default)]
pub struct CSpec {
    pub minus: bool,
    pub plus: bool,
    pub space: bool,
    pub alt: bool,
    pub zero: bool,
    pub width: usize,
    pub prec: Option<usize>,
    pub conv: u8,
}

/// Parse a complete conversion specification such as `%-08.3f`. Returns
/// `None` if `f` is not exactly one valid specification (glibc then prints
/// the text unchanged).
pub fn parse_cspec(f: &str) -> Option<CSpec> {
    let b = f.as_bytes();
    if b.first() != Some(&b'%') {
        return None;
    }
    let mut sp = CSpec::default();
    let mut i = 1;
    while i < b.len() {
        match b[i] {
            b'-' => sp.minus = true,
            b'+' => sp.plus = true,
            b' ' => sp.space = true,
            b'#' => sp.alt = true,
            b'0' => sp.zero = true,
            // thousands grouping and locale digits: no-ops in the C locale
            b'\'' | b'I' => {}
            _ => break,
        }
        i += 1;
    }
    let mut w: usize = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        w = w.saturating_mul(10).saturating_add((b[i] - b'0') as usize);
        i += 1;
    }
    sp.width = w;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let mut p: usize = 0;
        while i < b.len() && b[i].is_ascii_digit() {
            p = p.saturating_mul(10).saturating_add((b[i] - b'0') as usize);
            i += 1;
        }
        sp.prec = Some(p);
    }
    if i + 1 != b.len() {
        return None;
    }
    sp.conv = b[i];
    if !b"diouxXeEfFgGaAsc".contains(&sp.conv) {
        return None;
    }
    Some(sp)
}

fn pad(sp: &CSpec, prefix: &str, body: &str, zero_ok: bool) -> String {
    let len = prefix.len() + body.len();
    if len >= sp.width {
        return format!("{}{}", prefix, body);
    }
    let n = sp.width - len;
    if sp.minus {
        format!("{}{}{}", prefix, body, " ".repeat(n))
    } else if sp.zero && zero_ok {
        format!("{}{}{}", prefix, "0".repeat(n), body)
    } else {
        format!("{}{}{}", " ".repeat(n), prefix, body)
    }
}

/// `%d %i %o %u %x %X` (and `%c`) of a C `int`.
pub fn cfmt_int(sp: &CSpec, v: i32) -> String {
    // Fast path: signed decimal without precision or `#`, the shape of
    // `%d`/`%05d` in hot code. glibc pads with zeros after the sign.
    if matches!(sp.conv, b'd' | b'i') && sp.prec.is_none() && !sp.alt {
        let neg = v < 0;
        let mut m = (v as i64).unsigned_abs();
        let mut buf = [0u8; 10];
        let mut i = buf.len();
        loop {
            i -= 1;
            buf[i] = b'0' + (m % 10) as u8;
            m /= 10;
            if m == 0 {
                break;
            }
        }
        let digits = &buf[i..];
        let sign = if neg {
            '-'
        } else if sp.plus {
            '+'
        } else if sp.space {
            ' '
        } else {
            '\0'
        };
        let total = digits.len() + usize::from(sign != '\0');
        let width = sp.width;
        let mut out = String::with_capacity(width.max(total));
        if total >= width {
            if sign != '\0' {
                out.push(sign);
            }
            out.push_str(std::str::from_utf8(digits).unwrap());
        } else {
            let pad = width - total;
            if sp.minus {
                if sign != '\0' {
                    out.push(sign);
                }
                out.push_str(std::str::from_utf8(digits).unwrap());
                out.push_str(&" ".repeat(pad));
            } else if sp.zero {
                if sign != '\0' {
                    out.push(sign);
                }
                out.push_str(&"0".repeat(pad));
                out.push_str(std::str::from_utf8(digits).unwrap());
            } else {
                out.push_str(&" ".repeat(pad));
                if sign != '\0' {
                    out.push(sign);
                }
                out.push_str(std::str::from_utf8(digits).unwrap());
            }
        }
        return out;
    }
    let signed = matches!(sp.conv, b'd' | b'i');
    let mut digits = match sp.conv {
        b'd' | b'i' => (v as i64).unsigned_abs().to_string(),
        b'o' => format!("{:o}", v as u32),
        b'u' => (v as u32).to_string(),
        b'x' => format!("{:x}", v as u32),
        b'X' => format!("{:X}", v as u32),
        b'c' => {
            let c = char::from_u32(v as u32 & 0xff).unwrap_or('?');
            return pad(sp, "", &c.to_string(), false);
        }
        _ => v.to_string(),
    };
    if let Some(p) = sp.prec {
        if p == 0 && v == 0 {
            digits.clear();
        } else if digits.len() < p {
            digits = format!("{}{}", "0".repeat(p - digits.len()), digits);
        }
    }
    if sp.alt && sp.conv == b'o' && !digits.starts_with('0') {
        digits.insert(0, '0');
    }
    let mut prefix = String::new();
    if signed {
        if v < 0 {
            prefix.push('-');
        } else if sp.plus {
            prefix.push('+');
        } else if sp.space {
            prefix.push(' ');
        }
    }
    if sp.alt && v != 0 {
        if sp.conv == b'x' {
            prefix.push_str("0x");
        } else if sp.conv == b'X' {
            prefix.push_str("0X");
        }
    }
    pad(sp, &prefix, &digits, sp.prec.is_none())
}

/// `%a`: hexadecimal floating point, as glibc prints it.
fn fmt_hex_float(x: f64, prec: Option<usize>, upper: bool, alt: bool) -> String {
    let bits = x.to_bits();
    let bexp = ((bits >> 52) & 0x7ff) as i64;
    let mut mant = bits & ((1u64 << 52) - 1);
    let (mut lead, exp) =
        if bexp == 0 { if mant == 0 { (0u64, 0i64) } else { (0u64, -1022i64) } } else { (1u64, bexp - 1023) };
    let mut ndig = 13usize;
    if let Some(p) = prec {
        if p < 13 {
            let shift = (13 - p) * 4;
            let full = (lead << 52) | mant;
            let rem = full & ((1u64 << shift) - 1);
            let mut q = full >> shift;
            let half = 1u64 << (shift - 1);
            if rem > half || (rem == half && (q & 1) == 1) {
                q += 1;
            }
            let full = q << shift;
            lead = full >> 52;
            mant = full & ((1u64 << 52) - 1);
        }
        ndig = p;
    }
    let mut hex = format!("{:013x}", mant);
    match prec {
        None => {
            while hex.ends_with('0') {
                hex.pop();
            }
        }
        Some(p) if p <= 13 => hex.truncate(p),
        Some(p) => hex.push_str(&"0".repeat(p - 13)),
    }
    let _ = ndig;
    let mut s = format!("{}", lead);
    if !hex.is_empty() || alt {
        s.push('.');
        s.push_str(&hex);
    }
    s.push('p');
    s.push_str(&format!("{:+}", exp));
    if upper { s.to_uppercase() } else { s }
}

/// `%f %e %E %g %G %a %A` of a double.
pub fn cfmt_double(sp: &CSpec, x: f64) -> String {
    let neg = x.is_sign_negative() && !x.is_nan();
    let ax = x.abs();
    let upper = sp.conv.is_ascii_uppercase();
    let mut hex_prefix = "";
    let body = if !x.is_finite() {
        let s = if x.is_nan() { "nan" } else { "inf" };
        let s = if upper { s.to_uppercase() } else { s.to_string() };
        let mut prefix = String::new();
        if neg {
            prefix.push('-');
        } else if sp.plus {
            prefix.push('+');
        } else if sp.space {
            prefix.push(' ');
        }
        return pad(sp, &prefix, &s, false);
    } else {
        match sp.conv {
            b'f' | b'F' => {
                let p = sp.prec.unwrap_or(6);
                let mut s = robigo_syntax::cfmt::fmt_f(ax, p);
                if sp.alt && p == 0 {
                    s.push('.');
                }
                s
            }
            b'e' | b'E' => {
                let p = sp.prec.unwrap_or(6);
                let mut s = robigo_syntax::cfmt::fmt_e(ax, p, upper);
                if sp.alt && p == 0 {
                    if let Some(i) = s.find(['e', 'E']) {
                        s.insert(i, '.');
                    }
                }
                s
            }
            b'g' | b'G' => robigo_syntax::cfmt::fmt_g(ax, sp.prec.unwrap_or(6), sp.alt, upper),
            b'a' | b'A' => {
                hex_prefix = if upper { "0X" } else { "0x" };
                fmt_hex_float(ax, sp.prec, upper, sp.alt)
            }
            _ => robigo_syntax::cfmt::fmt_g(ax, 6, false, false),
        }
    };
    let mut prefix = String::new();
    if neg {
        prefix.push('-');
    } else if sp.plus {
        prefix.push('+');
    } else if sp.space {
        prefix.push(' ');
    }
    prefix.push_str(hex_prefix);
    pad(sp, &prefix, &body, true)
}

/// `%s`: precision truncates (in bytes, kept on a character boundary),
/// width pads in bytes as the C library does.
pub fn cfmt_str(sp: &CSpec, s: &str) -> String {
    if sp.width == 0 && sp.prec.is_none() && !sp.minus && !sp.plus && !sp.space && !sp.alt && !sp.zero {
        return s.to_string();
    }
    let mut s = s;
    if let Some(p) = sp.prec {
        if p < s.len() {
            let mut end = p;
            while !s.is_char_boundary(end) {
                end -= 1;
            }
            s = &s[..end];
        }
    }
    pad(sp, "", s, false)
}

/// Format one argument with a complete C specification; an invalid
/// specification is copied through unchanged, as glibc does.
pub enum CArg<'a> {
    Int(i32),
    Double(f64),
    Str(&'a str),
}

pub fn c_format(fmt: &str, arg: CArg) -> String {
    let Some(sp) = parse_cspec(fmt) else { return fmt.to_string() };
    match arg {
        CArg::Int(i) => match sp.conv {
            b'e' | b'E' | b'f' | b'F' | b'g' | b'G' | b'a' | b'A' => cfmt_double(&sp, i as f64),
            b's' => cfmt_str(&sp, &i.to_string()),
            _ => cfmt_int(&sp, i),
        },
        CArg::Double(x) => match sp.conv {
            b'd' | b'i' | b'o' | b'u' | b'x' | b'X' | b'c' => cfmt_int(&sp, x as i32),
            b's' => cfmt_str(&sp, &x.to_string()),
            _ => cfmt_double(&sp, x),
        },
        CArg::Str(s) => cfmt_str(&sp, s),
    }
}

// ----------------------------------------------------------------------
// sprintf

const MAXLINE: usize = 8192;
const MAXNARGS: usize = 100;

/// R: sprintf.c:findspec. Index of the conversion character after the
/// flags, width and precision.
fn findspec(s: &[u8]) -> usize {
    if s.first() != Some(&b'%') {
        return 0;
    }
    let mut p = 1;
    while p < s.len() {
        let c = s[p];
        if matches!(c, b'-' | b'+' | b' ' | b'#' | b'.' | b'*') || c.is_ascii_digit() {
            p += 1;
            continue;
        }
        break;
    }
    p
}

/// R: sprintf.c:checkfmt. `true` is an error: no conversion from `pattern`.
fn checkfmt(fmt: &str, pattern: &str) -> bool {
    let b = fmt.as_bytes();
    if b.first() != Some(&b'%') {
        return true;
    }
    let p = findspec(b);
    match b.get(p) {
        Some(c) => !pattern.as_bytes().contains(c),
        None => true,
    }
}

fn strcspn(s: &[u8], set: &[u8]) -> usize {
    s.iter().position(|c| set.contains(c)).unwrap_or(s.len())
}

fn my_sprintf(fmtp: &str, arg: CArg) -> RResult<String> {
    let out = c_format(fmtp, arg);
    if out.len() > MAXLINE {
        return Err(Flow::error(format!(
            "required resulting string length {} is greater than maximal {}",
            out.len(),
            MAXLINE
        )));
    }
    Ok(out)
}

fn recycle_error() -> Flow {
    Flow::error("arguments cannot be recycled to the same length")
}

fn check_maxlen(nfmt: usize, lens: &[usize]) -> RResult<usize> {
    let mut maxlen = nfmt;
    for &l in lens {
        maxlen = maxlen.max(l);
    }
    if maxlen % nfmt != 0 {
        return Err(recycle_error());
    }
    for &l in lens {
        if maxlen % l != 0 {
            return Err(recycle_error());
        }
    }
    Ok(maxlen)
}

/// Evaluate `fname(x)` in `env` (R constructs the call and evaluates it,
/// so S3 methods for `as.character` apply).
fn coerce_via(interp: &mut Interp, fname: &str, x: Value, env: &Env) -> RResult<Value> {
    let call = Value::lang(Value::sym(fname), vec![Arg::pos(x)]);
    interp.eval(&call, env)
}

/// Parse `%n$` / `%nn$` right after position `at` (which holds `%` or
/// `*`). Returns the argument index and the number of bytes to drop.
fn parse_dollar(fmt: &[u8], at: usize, nargs: usize) -> RResult<Option<(usize, usize)>> {
    let rest = &fmt[at..];
    if rest.len() > 3 && (b'1'..=b'9').contains(&rest[1]) {
        let v = (rest[1] - b'0') as usize;
        if rest[2] == b'$' {
            if v > nargs {
                return Err(Flow::error(format!("reference to non-existent argument {}", v)));
            }
            return Ok(Some((v - 1, 2)));
        } else if rest[2].is_ascii_digit() && rest[3] == b'$' {
            let v = 10 * v + (rest[2] - b'0') as usize;
            if v > nargs {
                return Err(Flow::error(format!("reference to non-existent argument {}", v)));
            }
            return Ok(Some((v - 1, 3)));
        }
    }
    Ok(None)
}

fn elt_len(v: &Value) -> usize {
    v.len()
}

// R: sprintf.c:do_sprintf
fn do_sprintf(interp: &mut Interp, _call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let mut it = args.into_iter().map(|a| a.value);
    let format = it.next().unwrap_or(Value::Null);
    let Value::Str(format) = format else {
        return Err(Flow::error("'fmt' is not a character vector"));
    };
    let nfmt = format.data.len();
    if nfmt == 0 {
        return Ok(Value::strs(vec![]));
    }
    let mut a: Vec<Value> = it.collect();
    let nargs = a.len();
    if nargs >= MAXNARGS {
        return Err(Flow::error(format!("only {} arguments are allowed", MAXNARGS)));
    }
    let mut used = vec![false; nargs];
    let mut lens = vec![0usize; nargs];
    for (i, ai) in a.iter().enumerate() {
        if matches!(ai, Value::Lang(_) | Value::Sym(_)) {
            return Err(Flow::error(format!("invalid type of argument[{}]: '{}'", i + 1, ai.type_name())));
        }
        lens[i] = elt_len(ai);
        if lens[i] == 0 {
            return Ok(Value::strs(vec![]));
        }
    }
    let mut maxlen = check_maxlen(nfmt, &lens)?;

    let mut ans: Vec<RStr> = Vec::new();
    let mut ns = 0;
    while ns < maxlen {
        let mut output = String::new();
        let format_string: &str = match &format.data[ns % nfmt] {
            Some(s) => s,
            None => "NA",
        };
        let fbytes = format_string.as_bytes();
        let n = fbytes.len();
        if n > MAXLINE {
            return Err(Flow::error(format!("'fmt' length exceeds maximal format length {}", MAXLINE)));
        }
        let mut cnt = 0usize;
        let mut cur = 0usize;
        while cur < n {
            let chunk;
            if fbytes[cur] == b'%' {
                if cur < n - 1 && fbytes[cur + 1] == b'%' {
                    chunk = 2;
                    output.push('%');
                } else {
                    chunk = strcspn(&fbytes[cur + 1..], b"diosfeEgGxXaA") + 2;
                    if cur + chunk > n {
                        return Err(Flow::error(format!(
                            "unrecognised format specification '{}'",
                            &format_string[cur..]
                        )));
                    }
                    let mut fmt: Vec<u8> = fbytes[cur..cur + chunk].to_vec();
                    let mut nthis: Option<usize> = None;
                    if fmt.len() > 3 {
                        if let Some((v, drop)) = parse_dollar(&fmt, 0, nargs)? {
                            nthis = Some(v);
                            fmt.drain(1..1 + drop);
                        }
                    }
                    let mut star_arg: Option<i32> = None;
                    if let Some(sp) = fmt.iter().position(|&c| c == b'*') {
                        let mut nstar: Option<usize> = None;
                        if fmt.len() - sp > 3 {
                            if let Some((v, drop)) = parse_dollar(&fmt, sp, nargs)? {
                                nstar = Some(v);
                                fmt.drain(sp + 1..sp + 1 + drop);
                            }
                        }
                        let nstar = match nstar {
                            Some(v) => v,
                            None => {
                                if cnt >= nargs {
                                    return Err(Flow::error("too few arguments"));
                                }
                                cnt += 1;
                                cnt - 1
                            }
                        };
                        if fmt[sp + 1..].contains(&b'*') {
                            return Err(Flow::error(
                                "at most one asterisk '*' is supported in each conversion specification",
                            ));
                        }
                        used[nstar] = true;
                        if ns == 0 {
                            if let Value::Real(_) = a[nstar] {
                                a[nstar] = crate::coerce::coerce_quiet(&a[nstar], VType::Int);
                            }
                        }
                        let this = &a[nstar];
                        let v = match this {
                            Value::Int(v) if !v.data.is_empty() => v.data[ns % v.data.len()],
                            _ => NA_INTEGER,
                        };
                        if v == NA_INTEGER {
                            return Err(Flow::error("argument for '*' conversion specification must be a number"));
                        }
                        star_arg = Some(v);
                    }
                    // fmt is ASCII up to the conversion except for user text
                    let fmt_s = String::from_utf8_lossy(&fmt).into_owned();
                    if fmt.last() == Some(&b'%') {
                        let bit = match star_arg {
                            Some(v) => c_format(&fmt_s.replacen('*', &v.to_string(), 1), CArg::Int(v)),
                            None => fmt_s.clone(),
                        };
                        output.push_str(&bit);
                    } else {
                        let nthis = match nthis {
                            Some(v) => v,
                            None => {
                                if cnt >= nargs {
                                    return Err(Flow::error("too few arguments"));
                                }
                                cnt += 1;
                                cnt - 1
                            }
                        };
                        used[nthis] = true;
                        let mut fmtp = match star_arg {
                            Some(v) => fmt_s.replacen('*', &v.to_string(), 1),
                            None => fmt_s,
                        };
                        if fmtp.len() > MAXLINE {
                            return Err(Flow::error(format!("'fmt' length exceeds maximal format length {}", MAXLINE)));
                        }
                        // Minimal coercion, only for the first row.
                        if ns == 0 {
                            let spec = fmtp.as_bytes().get(findspec(fmtp.as_bytes())).copied();
                            match spec {
                                Some(b'd' | b'i' | b'o' | b'x' | b'X') => {
                                    if let Value::Real(v) = &a[nthis] {
                                        let exactly_integer = v
                                            .data
                                            .iter()
                                            .all(|&r| is_na_real(r) || (r.is_finite() && (r as i32) as f64 == r));
                                        if exactly_integer {
                                            a[nthis] = crate::coerce::coerce_quiet(&a[nthis], VType::Int);
                                        }
                                    }
                                }
                                Some(b'a' | b'A' | b'e' | b'f' | b'g' | b'E' | b'G') => {
                                    if !matches!(a[nthis], Value::Real(_) | Value::Str(_)) {
                                        let x = a[nthis].clone();
                                        a[nthis] = coerce_via(interp, "as.double", x, env)?;
                                        let thislen = elt_len(&a[nthis]);
                                        if thislen == 0 {
                                            return Err(Flow::error("coercion has changed vector length to 0"));
                                        }
                                        let do_check = lens[nthis] == maxlen;
                                        lens[nthis] = thislen;
                                        if do_check && thislen < maxlen {
                                            maxlen = check_maxlen(nfmt, &lens)?;
                                        }
                                    }
                                }
                                Some(b's') => {
                                    if !matches!(a[nthis], Value::Str(_)) {
                                        let x = a[nthis].clone();
                                        a[nthis] = coerce_via(interp, "as.character", x, env)?;
                                        let thislen = elt_len(&a[nthis]);
                                        if thislen == 0 {
                                            return Err(Flow::error("coercion has changed vector length to 0"));
                                        }
                                        let do_check = lens[nthis] == maxlen;
                                        lens[nthis] = thislen;
                                        if do_check && thislen < maxlen {
                                            maxlen = check_maxlen(nfmt, &lens)?;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        let this = &a[nthis];
                        let thislen = elt_len(this);
                        if thislen == 0 {
                            return Err(Flow::error("coercion has changed vector length to 0"));
                        }
                        let bit = format_one(this, ns % thislen, &mut fmtp, interp)?;
                        output.push_str(&bit);
                    }
                }
            } else {
                chunk = fbytes[cur..].iter().position(|&c| c == b'%').unwrap_or(n - cur);
                output.push_str(&format_string[cur..cur + chunk]);
            }
            cur += chunk;
        }
        if ns == 0 {
            ans.reserve(maxlen);
        }
        ans.push(Some(output.into()));
        ns += 1;
    }

    let nunused = used.iter().filter(|u| !**u).count();
    if nunused > 0 {
        if nfmt == 1 {
            let f = format.data[0].as_deref().unwrap_or("NA");
            if nunused == 1 {
                interp.warning(format!("one argument not used by format '{}'", f))?;
            } else {
                interp.warning(format!("{} arguments not used by format '{}'", nunused, f))?;
            }
        } else if nunused == 1 {
            interp.warning("one argument not used by format")?;
        } else {
            interp.warning(format!("{} arguments not used by format", nunused))?;
        }
    }
    Ok(Value::strs(ans))
}

/// Format element `i` of `this` with the conversion `fmtp` (one row of
/// the type switch in do_sprintf).
fn format_one(this: &Value, i: usize, fmtp: &mut String, interp: &mut Interp) -> RResult<String> {
    let _ = interp;
    let invalid = |fmtp: &str, what: &str| Flow::error(format!("invalid format '{}'; {}", fmtp, what));
    let set_last_s = |f: &mut String| {
        f.pop();
        f.push('s');
    };
    match this {
        Value::Lgl(v) => {
            let x = v.data[i];
            if checkfmt(fmtp, "di") {
                return Err(invalid(fmtp, "use format %d or %i for logical objects"));
            }
            if x == NA_LOGICAL {
                set_last_s(fmtp);
                my_sprintf(fmtp, CArg::Str("NA"))
            } else {
                my_sprintf(fmtp, CArg::Int(x))
            }
        }
        Value::Int(v) => {
            let x = v.data[i];
            if checkfmt(fmtp, "dioxX") {
                return Err(invalid(fmtp, "use format %d, %i, %o, %x or %X for integer objects"));
            }
            if x == NA_INTEGER {
                set_last_s(fmtp);
                my_sprintf(fmtp, CArg::Str("NA"))
            } else {
                my_sprintf(fmtp, CArg::Int(x))
            }
        }
        Value::Real(v) => {
            let x = v.data[i];
            if checkfmt(fmtp, "aAfeEgG") {
                return Err(invalid(fmtp, "use format %f, %e, %g or %a for numeric objects"));
            }
            if x.is_finite() {
                my_sprintf(fmtp, CArg::Double(x))
            } else {
                if let Some(p) = fmtp.find('.') {
                    fmtp.truncate(p);
                    fmtp.push('s');
                } else {
                    set_last_s(fmtp);
                }
                let has_space = fmtp.contains(' ');
                let s = if is_na_real(x) {
                    if has_space { " NA" } else { "NA" }
                } else if x.is_nan() {
                    if has_space { " NaN" } else { "NaN" }
                } else if x > 0.0 {
                    if fmtp.contains('+') {
                        "+Inf"
                    } else if has_space {
                        " Inf"
                    } else {
                        "Inf"
                    }
                } else {
                    "-Inf"
                };
                my_sprintf(fmtp, CArg::Str(s))
            }
        }
        Value::Str(v) => {
            if checkfmt(fmtp, "s") {
                return Err(invalid(fmtp, "use format %s for character objects"));
            }
            let ss: &str = v.data[i].as_deref().unwrap_or("NA");
            if fmtp.as_bytes().get(1) != Some(&b's') {
                if ss.len() > MAXLINE {
                    interp.warning(format!("likely truncation of character string to {} characters", MAXLINE - 1))?;
                }
                let mut out = my_sprintf(fmtp, CArg::Str(ss))?;
                if out.len() > MAXLINE {
                    let mut end = MAXLINE;
                    while !out.is_char_boundary(end) {
                        end -= 1;
                    }
                    out.truncate(end);
                }
                Ok(out)
            } else {
                Ok(ss.to_string())
            }
        }
        _ => Err(Flow::error("unsupported type")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(fmt: &str, a: CArg) -> String {
        c_format(fmt, a)
    }

    #[test]
    fn ints() {
        assert_eq!(f("%5d", CArg::Int(42)), "   42");
        assert_eq!(f("%-5d|", CArg::Int(42)), "%-5d|");
        assert_eq!(f("%-5d", CArg::Int(42)), "42   ");
        assert_eq!(f("%05d", CArg::Int(-42)), "-0042");
        assert_eq!(f("%+d", CArg::Int(5)), "+5");
        assert_eq!(f("% d", CArg::Int(5)), " 5");
        assert_eq!(f("%.3d", CArg::Int(5)), "005");
        assert_eq!(f("%x", CArg::Int(255)), "ff");
        assert_eq!(f("%#X", CArg::Int(255)), "0XFF");
        assert_eq!(f("%#o", CArg::Int(8)), "010");
        assert_eq!(f("%x", CArg::Int(-1)), "ffffffff");
        assert_eq!(f("%.0d", CArg::Int(0)), "");
    }

    #[test]
    fn doubles() {
        assert_eq!(f("%.2f", CArg::Double(3.14159)), "3.14");
        assert_eq!(f("%10.3e", CArg::Double(12345.678)), " 1.235e+04");
        assert_eq!(f("%g", CArg::Double(1e-10)), "1e-10");
        assert_eq!(f("%#.0f", CArg::Double(3.0)), "3.");
        assert_eq!(f("%+.1f", CArg::Double(2.0)), "+2.0");
        assert_eq!(f("%08.2f", CArg::Double(-3.5)), "-0003.50");
        assert_eq!(f("%a", CArg::Double(1.5)), "0x1.8p+0");
        assert_eq!(f("%a", CArg::Double(1.0)), "0x1p+0");
        assert_eq!(f("%a", CArg::Double(0.0)), "0x0p+0");
        assert_eq!(f("%.1a", CArg::Double(1.0)), "0x1.0p+0");
        assert_eq!(f("%A", CArg::Double(-0.1)), "-0X1.999999999999AP-4");
    }

    #[test]
    fn strs() {
        assert_eq!(f("%5s", CArg::Str("ab")), "   ab");
        assert_eq!(f("%-5s", CArg::Str("ab")), "ab   ");
        assert_eq!(f("%.1s", CArg::Str("ab")), "a");
    }
}
