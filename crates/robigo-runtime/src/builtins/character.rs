//! String functions from GNU R's character.c (`nchar`, `substr`,
//! `toupper`, `chartr`, `make.names`, ...) together with the string
//! utilities of util.c (`basename`, `encodeString`, `Encoding`, ...),
//! raw.c (`utf8ToInt`, `intToUtf8`, `charToRaw`, `rawToChar`) and
//! unique.c (`make.unique`).
//!
//! robigo strings are always valid UTF-8 (R's "UTF-8" or ASCII CHARSXPs);
//! there are no latin1 or "bytes" encoded strings. Code paths of GNU R
//! for other encodings are therefore not ported.

use std::rc::Rc;

use super::{internal, prim};
use crate::coerce::{CoerceWarnings, as_integer, as_logical, coerce_vector};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::s;
use crate::value::*;

include!("character_widths.rs");

pub static DEFS: &[Builtin] = &[
    prim("nzchar", do_nzchar, -1),
    internal("nchar", do_nchar, 4),
    internal("substr", do_substr, 3),
    internal("substr<-", do_substrgets, 4),
    internal("startsWith", do_startswith, 2),
    internal("endsWith", do_endswith, 2),
    internal("abbreviate", do_abbrev, 3),
    internal("make.names", do_makenames, 2),
    internal("tolower", do_tolower, 1),
    internal("toupper", do_toupper, 1),
    internal("chartr", do_chartr, 3),
    internal("strtrim", do_strtrim, 2),
    internal("strtoi", do_strtoi, 2),
    internal("strrep", do_strrep, 2),
    internal("encodeString", do_encodestring, 5),
    internal("Encoding", do_encoding, 1),
    internal("setEncoding", do_setencoding, 2),
    prim("enc2native", do_enc2native, 1),
    prim("enc2utf8", do_enc2utf8, 1),
    internal("iconv", do_iconv, 6),
    internal("utf8ToInt", do_utf8toint, 1),
    internal("intToUtf8", do_inttoutf8, 3),
    internal("validUTF8", do_validutf8, 1),
    internal("validEnc", do_validenc, 1),
    internal("charToRaw", do_chartoraw, 1),
    internal("rawToChar", do_rawtochar, 2),
    internal("basename", do_basename, 1),
    internal("dirname", do_dirname, 1),
    internal("l10n_info", do_l10n_info, 0),
];

// ----------------------------------------------------------------------
// helpers

/// R: rlocale.c:Ri18n_wcwidth. Display width of a code point, using R's
/// own tables (GNU R is configured with `--with-internal-wcwidth`).
pub fn ri18n_wcwidth(c: char) -> usize {
    let c = c as u32;
    if c > 0x1F && c < 0x7F {
        return 1;
    }
    if let Some(w) = table_search(TABLE_WCWIDTH, c) {
        return w as usize;
    }
    if ZERO_WIDTH.binary_search_by(|&(a, b)| range_cmp(a, b, c)).is_ok() { 0 } else { 1 }
}

fn range_cmp(first: u32, last: u32, c: u32) -> std::cmp::Ordering {
    if last < c {
        std::cmp::Ordering::Less
    } else if first > c {
        std::cmp::Ordering::Greater
    } else {
        std::cmp::Ordering::Equal
    }
}

fn table_search(t: &[(u32, u32, u8)], c: u32) -> Option<u8> {
    t.binary_search_by(|&(a, b, _)| range_cmp(a, b, c)).ok().map(|i| t[i].2)
}

/// Display width of a string (sum of `Ri18n_wcwidth`).
pub fn str_width(s: &str) -> usize {
    s.chars().map(ri18n_wcwidth).sum()
}

/// `checkArity` for `.Internal`s (error() against the context call).
pub(crate) fn check_arity_internal(args: &[Arg], n: usize, name: &str) -> RResult<()> {
    if args.len() != n {
        return Err(Flow::error(format!(
            "{} argument{} passed to .Internal({}) which requires {}",
            args.len(),
            if args.len() == 1 { "" } else { "s" },
            name,
            n
        )));
    }
    Ok(())
}

/// R's `check1arg`: the first argument, if named, must match `formal`.
pub(crate) fn check1arg(args: &[Arg], call: &Value, formal: &str) -> RResult<()> {
    if let Some(a) = args.first() {
        if let Some(t) = a.tag {
            let n = t.name();
            if !n.is_empty() && !formal.starts_with(&*n) {
                return Err(Flow::error_call(
                    call.clone(),
                    format!("supplied argument name '{}' does not match '{}'", n, formal),
                ));
            }
        }
    }
    Ok(())
}

/// R: coerce.c:asBool2 (`asLogical2` with checking).
pub(crate) fn as_bool2(v: &Value, call: &Value) -> RResult<bool> {
    if v.is_atomic() && v.len() > 1 {
        return Err(Flow::error_call(call.clone(), format!("'length = {}' in coercion to 'logical(1)'", v.len())));
    }
    let l = as_logical(v);
    if l == NA_LOGICAL {
        return Err(Flow::error_call(call.clone(), "NA in coercion to boolean"));
    }
    Ok(l != 0)
}

fn is_factor(v: &Value) -> bool {
    matches!(v, Value::Int(_)) && v.inherits("factor")
}

/// `SHALLOW_DUPLICATE_ATTRIB(to, from)`.
pub(crate) fn copy_all_attrs(from: &Value, to: &mut Value) {
    if let Some(a) = from.attrs() {
        to.set_attrs(Some(Box::new(a.clone())));
    }
}

/// Coerce like R's `coerceVector(x, STRSXP)`, failing for non-vectors.
fn coerce_to_str(x: &Value) -> RResult<Value> {
    match x {
        Value::Str(_) => Ok(x.clone()),
        v if v.is_vector() || v.is_null() || matches!(v, Value::Sym(_) | Value::Lang(_) | Value::Pairlist(_)) => {
            let mut w = CoerceWarnings::default();
            coerce_vector(v, VType::Str, &mut w)
        }
        v => Err(Flow::error(format!("cannot coerce type '{}' to vector of type 'character'", v.type_name()))),
    }
}

fn str_data(v: &Value) -> &[RStr] {
    match v {
        Value::Str(s) => &s.data,
        _ => &[],
    }
}

fn int_data(v: &Value) -> Option<&[i32]> {
    match v {
        Value::Int(s) => Some(&s.data),
        _ => None,
    }
}

/// Simple (one-to-one) case mapping as glibc's `towupper`/`towlower`.
pub fn simple_upper(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_uppercase();
    }
    let mut it = c.to_uppercase();
    match (it.next(), it.next()) {
        (Some(u), None) => u,
        _ => match c {
            '\u{1FB3}' => '\u{1FBC}',
            '\u{1FC3}' => '\u{1FCC}',
            '\u{1FF3}' => '\u{1FFC}',
            _ => c,
        },
    }
}

pub fn simple_lower(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    let mut it = c.to_lowercase();
    match (it.next(), it.next()) {
        (Some(l), None) => l,
        _ => match c {
            '\u{130}' => 'i',
            _ => c,
        },
    }
}

/// glibc's `iswalpha` in a UTF-8 locale (letters, and digits other than
/// ASCII ones).
pub fn is_walpha(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphabetic();
    }
    c.is_alphabetic() || c.is_numeric()
}

pub fn is_walnum(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphanumeric();
    }
    c.is_alphabetic() || c.is_numeric()
}

// ----------------------------------------------------------------------
// nzchar, nchar

// R: character.c:do_nzchar
fn do_nzchar(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let nargs = args.len();
    if !(1..=2).contains(&nargs) {
        return Err(Flow::error_call(
            call.clone(),
            format!(
                "{} argument{} passed to '{}' which requires {} to {}",
                nargs,
                if nargs == 1 { "" } else { "s" },
                "nzchar",
                1,
                2
            ),
        ));
    }
    check1arg(&args, call, "x")?;
    if is_factor(&args[0].value) {
        return Err(Flow::error("'nzchar()' requires a character vector"));
    }
    let x = coerce_to_str(&args[0].value)?;
    let mut keep_na = false;
    if nargs > 1 {
        let k = as_logical(&args[1].value);
        keep_na = k != NA_LOGICAL && k != 0;
    }
    let out: Vec<Lgl> = str_data(&x)
        .iter()
        .map(|e| match e {
            None => {
                if keep_na {
                    NA_LOGICAL
                } else {
                    TRUE
                }
            }
            Some(s) => (!s.is_empty()) as Lgl,
        })
        .collect();
    Ok(Value::lgls(out))
}

#[derive(Clone, Copy, PartialEq)]
pub enum NcharType {
    Bytes,
    Chars,
    Width,
}

/// R: character.c:R_nchar
pub fn r_nchar(s: &RStr, ty: NcharType, keep_na: bool) -> i32 {
    match s {
        None => {
            if keep_na {
                NA_INTEGER
            } else {
                2
            }
        }
        Some(s) => match ty {
            NcharType::Bytes => s.len() as i32,
            NcharType::Chars => s.chars().count() as i32,
            NcharType::Width => str_width(s) as i32,
        },
    }
}

// R: character.c:do_nchar
fn do_nchar(interp: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let nargs = args.len();
    if !(3..=4).contains(&nargs) {
        return Err(Flow::error(format!(
            "{} argument{} passed to '{}' which requires {} to {}",
            nargs,
            if nargs == 1 { "" } else { "s" },
            "nchar",
            3,
            4
        )));
    }
    if let Some(r) = interp.try_dispatch("nchar", call, &args, env)? {
        return Ok(r);
    }
    if is_factor(&args[0].value) {
        return Err(Flow::error("'nchar()' requires a character vector"));
    }
    let x = coerce_to_str(&args[0].value)?;
    let ty = match &args[1].value {
        Value::Str(t) if t.data.len() == 1 => t.data[0].clone().unwrap_or_else(|| Rc::from("NA")),
        _ => return Err(Flow::error("invalid 'type' argument")),
    };
    if ty.is_empty() {
        return Err(Flow::error("invalid 'type' argument"));
    }
    let ty = if "bytes".starts_with(&*ty) {
        NcharType::Bytes
    } else if "chars".starts_with(&*ty) {
        NcharType::Chars
    } else if "width".starts_with(&*ty) {
        NcharType::Width
    } else {
        return Err(Flow::error("invalid 'type' argument"));
    };
    let keep_na = if nargs >= 4 {
        let k = as_logical(&args[3].value);
        if k == NA_LOGICAL { ty != NcharType::Width } else { k != 0 }
    } else {
        ty != NcharType::Width
    };
    let out: Vec<i32> = str_data(&x).iter().map(|e| r_nchar(e, ty, keep_na)).collect();
    let mut ans = Value::ints(out);
    for sym in [s::NAMES, s::DIM, s::DIMNAMES] {
        if let Some(a) = x.get_attr(sym) {
            ans.set_attr_raw(sym, a);
        }
    }
    Ok(ans)
}

// ----------------------------------------------------------------------
// substr, substr<-

/// Byte range of characters `sa..=so` (1-based, `sa >= 1`, `sa <= so`).
fn char_range(s: &str, sa: i32, so: i32) -> (usize, usize) {
    let mut from = s.len();
    let mut to = s.len();
    for (i, (b, _)) in s.char_indices().enumerate() {
        let i = i as i64 + 1;
        if i == sa as i64 {
            from = b;
        }
        if i == so as i64 + 1 {
            to = b;
            break;
        }
    }
    if from > to {
        from = to;
    }
    (from, to)
}

// R: character.c:do_substr
fn do_substr(_: &mut Interp, _call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 3, "substr")?;
    let x = &args[0].value;
    let Value::Str(xv) = x else {
        return Err(Flow::error("extracting substrings from a non-character object"));
    };
    let mut out = Vec::with_capacity(xv.data.len());
    if !xv.data.is_empty() {
        let (Some(sa), Some(so)) = (int_data(&args[1].value), int_data(&args[2].value)) else {
            return Err(Flow::error("invalid substring arguments"));
        };
        if sa.is_empty() || so.is_empty() {
            return Err(Flow::error("invalid substring arguments"));
        }
        for (i, el) in xv.data.iter().enumerate() {
            let mut start = sa[i % sa.len()];
            let stop = so[i % so.len()];
            let Some(el) = el else {
                out.push(None);
                continue;
            };
            if start == NA_INTEGER || stop == NA_INTEGER {
                out.push(None);
                continue;
            }
            if start < 1 {
                start = 1;
            }
            if start > stop {
                out.push(Some(Rc::from("")));
            } else {
                let (a, b) = char_range(el, start, stop);
                out.push(Some(Rc::from(&el[a..b])));
            }
        }
    }
    let mut ans = Value::strs(out);
    copy_all_attrs(x, &mut ans);
    Ok(ans)
}

// R: character.c:do_substrgets (UTF-8 branch of substrset)
fn do_substrgets(_: &mut Interp, _call: &Value, mut args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 4, "substr<-")?;
    let x = std::mem::replace(&mut args[0].value, Value::Null);
    let Value::Str(xv) = &x else {
        return Err(Flow::error("replacing substrings in a non-character object"));
    };
    let len = xv.data.len();
    let mut out = Vec::with_capacity(len);
    if len > 0 {
        let (Some(sa), Some(so)) = (int_data(&args[1].value), int_data(&args[2].value)) else {
            return Err(Flow::error("invalid substring arguments"));
        };
        if sa.is_empty() || so.is_empty() {
            return Err(Flow::error("invalid substring arguments"));
        }
        let value = match &args[3].value {
            Value::Str(v) if !v.data.is_empty() => v.data.clone(),
            _ => return Err(Flow::error("invalid value")),
        };
        for i in 0..len {
            let el = &xv.data[i];
            let v_el = &value[i % value.len()];
            let mut start = sa[i % sa.len()];
            let mut stop = so[i % so.len()];
            let (Some(el), Some(v_el)) = (el, v_el) else {
                out.push(None);
                continue;
            };
            if start == NA_INTEGER || stop == NA_INTEGER {
                out.push(None);
                continue;
            }
            let slen = el.len() as i32;
            if start < 1 {
                start = 1;
            }
            if stop > slen {
                stop = slen;
            }
            if start > stop {
                out.push(Some(el.clone()));
                continue;
            }
            // Replace characters start..=stop of el by the leading
            // characters of v_el, never changing the number of characters.
            let mut buf = String::with_capacity(el.len() + v_el.len());
            let mut xc = el.chars();
            for _ in 1..start {
                match xc.next() {
                    Some(c) => buf.push(c),
                    None => break,
                }
            }
            let mut vc = v_el.chars();
            let mut k = start;
            let mut rest = xc.clone();
            while k <= stop {
                let Some(_) = rest.clone().next() else { break };
                let Some(nc) = vc.next() else { break };
                rest.next();
                buf.push(nc);
                k += 1;
            }
            buf.extend(rest);
            out.push(Some(Rc::from(buf.as_str())));
        }
    }
    let mut ans = Value::strs(out);
    copy_all_attrs(&x, &mut ans);
    Ok(ans)
}

// ----------------------------------------------------------------------
// startsWith, endsWith

fn starts_ends(args: &[Arg], ends: bool) -> RResult<Value> {
    check_arity_internal(args, 2, if ends { "endsWith" } else { "startsWith" })?;
    let (Value::Str(x), Value::Str(fix)) = (&args[0].value, &args[1].value) else {
        return Err(Flow::error("non-character object(s)"));
    };
    let (n1, n2) = (x.data.len(), fix.data.len());
    let n = if n1 > 0 && n2 > 0 { n1.max(n2) } else { 0 };
    let out: Vec<Lgl> = (0..n)
        .map(|i| match (&x.data[i % n1], &fix.data[i % n2]) {
            (Some(a), Some(b)) => {
                if ends {
                    a.as_bytes().ends_with(b.as_bytes()) as Lgl
                } else {
                    a.as_bytes().starts_with(b.as_bytes()) as Lgl
                }
            }
            _ => NA_LOGICAL,
        })
        .collect();
    Ok(Value::lgls(out))
}

fn do_startswith(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    starts_ends(&args, false)
}

fn do_endswith(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    starts_ends(&args, true)
}

// ----------------------------------------------------------------------
// abbreviate

const VOWELS: &[u32] = &[
    0x61, 0x65, 0x69, 0x6f, 0x75, 0xe0, 0xe1, 0x2e, 0xe3, 0xe4, 0xe5, 0xe8, 0xe9, 0xea, 0xeb, 0xec, 0xed, 0xee, 0xef,
    0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf8, 0xf9, 0xfa, 0xfb, 0xfc, 0x101, 0x103, 0x105, 0x113, 0x115, 0x117, 0x118, 0x11b,
    0x129, 0x12b, 0x12d, 0x12f, 0x131, 0x14d, 0x14f, 0x151, 0x169, 0x16b, 0x16d, 0x16f, 0x171, 0x173,
];

fn c_isspace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r')
}

fn w_isspace(c: char) -> bool {
    c_isspace(c) || matches!(c as u32, 0x1680 | 0x2000..=0x2006 | 0x2008..=0x200A | 0x2028 | 0x2029 | 0x205F | 0x3000)
}

/// R: character.c:stripchars / wstripchars (on a vector of chars).
fn stripchars(input: &str, minlen: usize, usecl: bool, wide: bool) -> String {
    let mut s: Vec<char> = input.chars().collect();
    let isspace = |c: char| if wide { w_isspace(c) } else { c_isspace(c) };
    let lc_vowel = |c: char| if wide { VOWELS.contains(&(c as u32)) } else { matches!(c, 'a' | 'e' | 'i' | 'o' | 'u') };
    let islower = |c: char| (c as u32) < 0x80 && c.is_ascii_lowercase();
    // s behaves like a NUL-terminated buffer: strlen is s.len().
    let firstchar = |s: &Vec<char>, i: usize| isspace(s[i - 1]);
    let lastchar = |s: &Vec<char>, i: usize| !isspace(s[i - 1]) && (i + 1 >= s.len() || isspace(s[i + 1]));
    let mut nspace = 0usize;
    'done: {
        if s.len() < minlen {
            break 'done;
        }
        let mut j = true;
        let mut i = s.len() as isize - 1;
        while i > 0 {
            let iu = i as usize;
            if iu < s.len() && isspace(s[iu]) {
                if j {
                    s.truncate(iu);
                } else {
                    nspace += 1;
                }
            } else {
                j = false;
            }
            if s.len() - nspace <= minlen {
                break 'done;
            }
            i -= 1;
        }
        if usecl {
            type Pred<'a> = Box<dyn Fn(&Vec<char>, usize) -> bool + 'a>;
            let passes: [Pred; 4] = [
                Box::new(|s, i| lc_vowel(s[i]) && lastchar(s, i)),
                Box::new(|s, i| lc_vowel(s[i]) && !firstchar(s, i)),
                Box::new(|s, i| islower(s[i]) && lastchar(s, i)),
                Box::new(|s, i| islower(s[i]) && !firstchar(s, i)),
            ];
            for pred in passes.iter() {
                let mut i = s.len() as isize - 1;
                while i > 0 {
                    let iu = i as usize;
                    if iu < s.len() && pred(&s, iu) {
                        s.remove(iu);
                    }
                    if s.len() - nspace <= minlen {
                        break 'done;
                    }
                    i -= 1;
                }
            }
        }
        let mut i = s.len() as isize - 1;
        while i > 0 {
            let iu = i as usize;
            if iu < s.len() && !firstchar(&s, iu) && !isspace(s[iu]) {
                s.remove(iu);
            }
            if s.len() - nspace <= minlen {
                break 'done;
            }
            i -= 1;
        }
    }
    let upper = s.len();
    if upper > minlen {
        let mut i = upper as isize - 1;
        while i > 0 {
            let iu = i as usize;
            if iu < s.len() && isspace(s[iu]) {
                s.remove(iu);
            }
            i -= 1;
        }
    }
    s.into_iter().collect()
}

// R: character.c:do_abbrev
fn do_abbrev(interp: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 3, "abbreviate")?;
    let x = &args[0].value;
    let Value::Str(xv) = x else {
        return Err(Flow::error("the first argument must be a character vector"));
    };
    let minlen = as_integer(&args[1].value);
    if minlen == NA_INTEGER {
        return Err(Flow::error("invalid 'minlength' argument"));
    }
    let usecl = as_logical(&args[2].value);
    if usecl == NA_LOGICAL {
        return Err(Flow::error("invalid 'use.classes' argument"));
    }
    let minlen = minlen.max(0) as usize;
    let mut warn = false;
    let out: Vec<RStr> = xv
        .data
        .iter()
        .map(|el| match el {
            None => None,
            Some(s) => {
                if s.is_ascii() {
                    if s.len() > minlen {
                        Some(Rc::from(stripchars(s, minlen, usecl != 0, false).as_str()))
                    } else {
                        Some(s.clone())
                    }
                } else if s.chars().count() > minlen {
                    warn = true;
                    Some(Rc::from(stripchars(s, minlen, usecl != 0, true).as_str()))
                } else {
                    Some(s.clone())
                }
            }
        })
        .collect();
    if usecl != 0 && warn {
        interp.warning("abbreviate used with non-ASCII chars")?;
    }
    let mut ans = Value::strs(out);
    copy_all_attrs(x, &mut ans);
    Ok(ans)
}

// ----------------------------------------------------------------------
// make.names, make.unique

/// R: gram.y:isValidName (in a UTF-8 locale).
pub fn is_valid_name(name: &str) -> bool {
    let mut it = name.chars();
    let Some(c0) = it.next() else { return false };
    if c0 != '.' && !is_walpha(c0) {
        return false;
    }
    if c0 == '.' && name.as_bytes().get(1).is_some_and(|b| b.is_ascii_digit()) {
        return false;
    }
    if !it.all(|c| is_walnum(c) || c == '.' || c == '_') {
        return false;
    }
    if name == "..." {
        return true;
    }
    !matches!(
        name,
        "NULL"
            | "NA"
            | "TRUE"
            | "FALSE"
            | "Inf"
            | "NaN"
            | "NA_integer_"
            | "NA_real_"
            | "NA_character_"
            | "NA_complex_"
            | "function"
            | "while"
            | "repeat"
            | "for"
            | "if"
            | "in"
            | "else"
            | "next"
            | "break"
    )
}

// R: character.c:do_makenames
fn do_makenames(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 2, "make.names")?;
    let Value::Str(names) = &args[0].value else {
        return Err(Flow::error("non-character names"));
    };
    let allow = as_logical(&args[1].value);
    if allow == NA_LOGICAL {
        return Err(Flow::error("invalid 'allow_' value"));
    }
    let out: Vec<RStr> = names
        .data
        .iter()
        .map(|el| {
            let this: &str = el.as_deref().unwrap_or("NA");
            let mut chars = this.chars();
            let need_prefix = match chars.next() {
                // !isalpha('\0')
                None => true,
                Some('.') => chars.next().is_some_and(|c| c.is_ascii_digit()),
                Some(c) => !is_walpha(c),
            };
            let mut tmp = String::with_capacity(this.len() + 2);
            if need_prefix {
                tmp.push('X');
            }
            for c in this.chars() {
                if c == '.' || (allow != 0 && c == '_') || is_walnum(c) {
                    tmp.push(c);
                } else {
                    tmp.push('.');
                }
            }
            if !is_valid_name(&tmp) {
                tmp.push('.');
            }
            Some(Rc::from(tmp.as_str()))
        })
        .collect();
    Ok(Value::strs(out))
}

// ----------------------------------------------------------------------
// tolower, toupper, chartr

fn case_map(args: &[Arg], upper: bool) -> RResult<Value> {
    check_arity_internal(args, 1, if upper { "toupper" } else { "tolower" })?;
    let x = &args[0].value;
    let Value::Str(xv) = x else {
        return Err(Flow::error("non-character argument"));
    };
    let out: Vec<RStr> = xv
        .data
        .iter()
        .map(|e| {
            e.as_ref().map(|s| {
                let t: String =
                    if upper { s.chars().map(simple_upper).collect() } else { s.chars().map(simple_lower).collect() };
                Rc::from(t.as_str())
            })
        })
        .collect();
    let mut ans = Value::strs(out);
    copy_all_attrs(x, &mut ans);
    Ok(ans)
}

fn do_tolower(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    case_map(&args, false)
}

fn do_toupper(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    case_map(&args, true)
}

/// R: character.c:wtr_build_spec, expanded to the character sequence.
fn tr_spec(s: &str) -> RResult<Vec<char>> {
    let w: Vec<char> = s.chars().collect();
    let len = w.len();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 2 < len {
        if w[i + 1] == '-' {
            if w[i] > w[i + 2] {
                return Err(Flow::error(format!("decreasing range specification ('{}-{}')", w[i], w[i + 2])));
            }
            let (a, b) = (w[i] as u32, w[i + 2] as u32);
            for c in a..=b {
                if let Some(c) = char::from_u32(c) {
                    out.push(c);
                }
            }
            i += 3;
        } else {
            out.push(w[i]);
            i += 1;
        }
    }
    while i < len {
        out.push(w[i]);
        i += 1;
    }
    Ok(out)
}

// R: character.c:do_chartr
fn do_chartr(interp: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 3, "chartr")?;
    let old = match &args[0].value {
        Value::Str(v) if !v.data.is_empty() && v.data[0].is_some() => v.clone(),
        _ => return Err(Flow::error("invalid 'old' argument")),
    };
    if old.data.len() > 1 {
        interp.warning("argument 'old' has length > 1 and only the first element will be used")?;
    }
    let new = match &args[1].value {
        Value::Str(v) if !v.data.is_empty() && v.data[0].is_some() => v.clone(),
        _ => return Err(Flow::error("invalid 'new' argument")),
    };
    if new.data.len() > 1 {
        interp.warning("argument 'new' has length > 1 and only the first element will be used")?;
    }
    let x = &args[2].value;
    let Value::Str(xv) = x else {
        return Err(Flow::error("invalid 'x' argument"));
    };
    let o = tr_spec(old.data[0].as_deref().unwrap())?;
    let nw = tr_spec(new.data[0].as_deref().unwrap())?;
    if o.len() > nw.len() {
        return Err(Flow::error("'old' is longer than 'new'"));
    }
    // later specifications of the same character win (R's COMPRESS)
    let mut table: std::collections::HashMap<char, char> = std::collections::HashMap::new();
    for (a, b) in o.iter().zip(nw.iter()) {
        table.insert(*a, *b);
    }
    let out: Vec<RStr> = xv
        .data
        .iter()
        .map(|e| {
            e.as_ref().map(|s| {
                let t: String = s.chars().map(|c| *table.get(&c).unwrap_or(&c)).collect();
                Rc::from(t.as_str())
            })
        })
        .collect();
    let mut ans = Value::strs(out);
    copy_all_attrs(x, &mut ans);
    Ok(ans)
}

// ----------------------------------------------------------------------
// strtrim, strtoi, strrep

// R: character.c:do_strtrim
fn do_strtrim(interp: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 2, "strtrim")?;
    let x = &args[0].value;
    let Value::Str(xv) = x else {
        return Err(Flow::error("strtrim() requires a character vector"));
    };
    let len = xv.data.len();
    let mut out = Vec::with_capacity(len);
    if len > 0 {
        let width = interp.coerce(&args[1].value, VType::Int)?;
        let w = int_data(&width).unwrap_or(&[]).to_vec();
        let nw = w.len();
        if nw == 0 || (nw < len && len % nw != 0) {
            return Err(Flow::error("invalid 'width' argument"));
        }
        if w.iter().any(|&v| v == NA_INTEGER || v < 0) {
            return Err(Flow::error("invalid 'width' argument"));
        }
        for (i, el) in xv.data.iter().enumerate() {
            let Some(s) = el else {
                out.push(None);
                continue;
            };
            let wi = w[i % nw] as usize;
            let mut wsum = 0usize;
            let mut buf = String::new();
            for c in s.chars() {
                wsum += ri18n_wcwidth(c);
                if wsum <= wi {
                    buf.push(c);
                } else {
                    break;
                }
            }
            out.push(Some(Rc::from(buf.as_str())));
        }
    }
    let mut ans = Value::strs(out);
    copy_all_attrs(x, &mut ans);
    Ok(ans)
}

/// C `strtol` on the whole string; `None` unless all of it is consumed
/// and the value fits an `int` (R: character.c:strtoi).
fn c_strtoi(s: &str, base: u32) -> Option<i32> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r') {
        i += 1;
    }
    let mut neg = false;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        neg = b[i] == b'-';
        i += 1;
    }
    let mut base = base;
    let has_0x = i + 1 < b.len() && b[i] == b'0' && (b[i + 1] == b'x' || b[i + 1] == b'X');
    if base == 0 {
        if has_0x && i + 2 < b.len() && (b[i + 2] as char).is_digit(16) {
            base = 16;
            i += 2;
        } else if i < b.len() && b[i] == b'0' {
            base = 8;
        } else {
            base = 10;
        }
    } else if base == 16 && has_0x && i + 2 < b.len() && (b[i + 2] as char).is_digit(16) {
        i += 2;
    }
    let start = i;
    let mut v: i64 = 0;
    let mut overflow = false;
    while i < b.len() {
        let Some(d) = (b[i] as char).to_digit(base) else { break };
        v = v.saturating_mul(base as i64).saturating_add(d as i64);
        if v > (1i64 << 40) {
            overflow = true;
        }
        i += 1;
    }
    if i == start || i != b.len() || overflow {
        return None;
    }
    let v = if neg { -v } else { v };
    if v > i32::MAX as i64 || v < i32::MIN as i64 + 1 {
        // INT_MIN itself is NA_INTEGER in R
        if v == i32::MIN as i64 {
            return Some(NA_INTEGER);
        }
        return None;
    }
    Some(v as i32)
}

// R: character.c:do_strtoi
fn do_strtoi(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 2, "strtoi")?;
    let base = match &args[1].value {
        Value::Int(b) if !b.data.is_empty() => b.data[0],
        _ => return Err(Flow::error("invalid 'base' argument")),
    };
    if base != 0 && !(2..=36).contains(&base) {
        return Err(Flow::error("invalid 'base' argument"));
    }
    let out: Vec<i32> = str_data(&args[0].value)
        .iter()
        .map(|e| match e {
            Some(s) if !s.is_empty() => c_strtoi(s, base as u32).unwrap_or(NA_INTEGER),
            _ => NA_INTEGER,
        })
        .collect();
    Ok(Value::ints(out))
}

// R: character.c:do_strrep
fn do_strrep(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 2, "strrep")?;
    let x = &args[0].value;
    let xs = str_data(x);
    let n = int_data(&args[1].value).unwrap_or(&[]);
    let (nx, nn) = (xs.len(), n.len());
    if nx == 0 || nn == 0 {
        return Ok(Value::strs(vec![]));
    }
    let ns = nx.max(nn);
    let mut out = Vec::with_capacity(ns);
    for i in 0..ns {
        let el = &xs[i % nx];
        let ni = n[i % nn];
        match el {
            Some(s) if ni != NA_INTEGER => {
                if ni < 0 {
                    return Err(Flow::error("invalid 'times' value"));
                }
                if (s.len() as f64) * (ni as f64) > i32::MAX as f64 {
                    return Err(Flow::error("R character strings are limited to 2^31-1 bytes"));
                }
                out.push(Some(Rc::from(s.repeat(ni as usize).as_str())));
            }
            _ => out.push(None),
        }
    }
    let mut ans = Value::strs(out);
    if ns == nx {
        if let Some(nm) = x.get_attr(s::NAMES) {
            ans.set_attr_raw(s::NAMES, nm);
        }
    }
    Ok(ans)
}

// ----------------------------------------------------------------------
// encodeString, Encoding, enc2*, iconv

// R: util.c:do_encodeString
fn do_encodestring(interp: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 5, "encodeString")?;
    let x = &args[0].value;
    let Value::Str(xv) = x else {
        return Err(Flow::error("a character vector argument expected"));
    };
    let mut w = if args[1].value.is_null() { NA_INTEGER } else { as_integer(&args[1].value) };
    if w != NA_INTEGER && w < 0 {
        return Err(Flow::error("invalid 'width' value"));
    }
    let q = match &args[2].value {
        Value::Str(q) if q.data.len() == 1 => q.data[0].clone().unwrap_or_else(|| Rc::from("NA")),
        _ => return Err(Flow::error("invalid 'quote' value")),
    };
    let quote = q.chars().next();
    if q.chars().count() > 1 {
        interp.warning("only the first character of 'quote' will be used")?;
    }
    let justify = as_integer(&args[3].value);
    if justify == NA_INTEGER || !(0..=3).contains(&justify) {
        return Err(Flow::error("invalid 'justify' value"));
    }
    if justify == 3 {
        w = 0;
    }
    let na = as_logical(&args[4].value);
    if na == NA_LOGICAL {
        return Err(Flow::error("invalid 'na.encode' value"));
    }
    let pp = interp.print_params();
    if w == NA_INTEGER && justify < 3 {
        let qw = if quote.is_some() { 2 } else { 0 };
        let mut ww = 0usize;
        for e in &xv.data {
            if na != 0 || e.is_some() {
                // R's Rstrlen: escaped width without the quotes; "NA" for NA
                let l = match e {
                    None => 2,
                    Some(s) => escaped_width(s, quote),
                };
                ww = ww.max(l);
            }
        }
        w = (ww + qw) as i32;
    }
    let just = match justify {
        0 => crate::format::Justify::Left,
        1 => crate::format::Justify::Right,
        2 => crate::format::Justify::Centre,
        _ => crate::format::Justify::None,
    };
    let out: Vec<RStr> = xv
        .data
        .iter()
        .map(|e| {
            if na != 0 || e.is_some() {
                Some(Rc::from(encode_string_q(e, w.max(0) as usize, quote, just, &pp).as_str()))
            } else {
                None
            }
        })
        .collect();
    let mut ans = Value::strs(out);
    copy_all_attrs(x, &mut ans);
    Ok(ans)
}

/// `EncodeString`'s escaping without the surrounding quotes. R only escapes
/// `'`, `"` and `` ` `` (with a backslash) when they are the quote character
/// itself (`// R: printutils.c:EncodeString`, the `case L'\''` arm); any other
/// quote character only delimits the result.
fn escape_body(s: &str, quote: Option<char>) -> String {
    use crate::format::escape_string;
    match quote {
        Some(q) if q == '"' || q == '\'' || q == '`' => {
            let with = escape_string(s, Some(q));
            with[1..with.len() - 1].to_string()
        }
        _ => escape_string(s, None),
    }
}

/// `Rstrlen` of the escaped form without the quotes.
fn escaped_width(s: &str, quote: Option<char>) -> usize {
    crate::format::str_width(&escape_body(s, quote))
}

/// `EncodeString` with padding (`// R: printutils.c:EncodeString`), with R's
/// quote-character rule (see `escape_body`).
fn encode_string_q(
    s: &RStr,
    w: usize,
    quote: Option<char>,
    justify: crate::format::Justify,
    pp: &crate::format::PrintParams,
) -> String {
    let (body, quoted) = match s {
        None => (if quote.is_some() { pp.na_string.clone() } else { pp.na_string_noquote.clone() }, 0),
        Some(s) => match quote {
            Some(q) => (format!("{q}{}{q}", escape_body(s, Some(q))), 2),
            None => (escape_body(s, None), 0),
        },
    };
    let bw = crate::format::str_width(&body) - quoted;
    let pad = w.saturating_sub(bw + quoted);
    if pad == 0 || justify == crate::format::Justify::None {
        return body;
    }
    match justify {
        crate::format::Justify::Left => format!("{}{}", body, " ".repeat(pad)),
        crate::format::Justify::Right => format!("{}{}", " ".repeat(pad), body),
        crate::format::Justify::Centre => {
            let l = pad / 2;
            format!("{}{}{}", " ".repeat(l), body, " ".repeat(pad - l))
        }
        crate::format::Justify::None => body,
    }
}

fn str_is_ascii(s: &RStr) -> bool {
    s.as_deref().is_none_or(|s| s.is_ascii())
}

// R: util.c:do_encoding
fn do_encoding(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 1, "Encoding")?;
    let Value::Str(x) = &args[0].value else {
        return Err(Flow::error("a character vector argument expected"));
    };
    Ok(Value::strs_from(x.data.iter().map(|e| if str_is_ascii(e) { "unknown" } else { "UTF-8" })))
}

// R: util.c:do_setencoding
fn do_setencoding(_: &mut Interp, _: &Value, mut args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 2, "setEncoding")?;
    let x = std::mem::replace(&mut args[0].value, Value::Null);
    let Value::Str(xv) = &x else {
        return Err(Flow::error("a character vector argument expected"));
    };
    let Value::Str(enc) = &args[1].value else {
        return Err(Flow::error("a character vector 'value' expected"));
    };
    if enc.data.is_empty() {
        return Err(Flow::error("'value' must be of positive length"));
    }
    for (i, e) in xv.data.iter().enumerate() {
        let this = enc.data[i % enc.data.len()].as_deref().unwrap_or("NA");
        if e.is_some() && !str_is_ascii(e) && (this == "latin1" || this == "bytes") {
            return Err(Flow::error(format!("robigo: marking strings as \"{}\" is not implemented yet", this)));
        }
    }
    Ok(x)
}

fn enc2(call: &Value, args: Vec<Arg>, name: &str) -> RResult<Value> {
    super::check_arity(&args, 1, name, call)?;
    check1arg(&args, call, "x")?;
    match &args[0].value {
        Value::Str(_) => Ok(args.into_iter().next().unwrap().value),
        _ => Err(Flow::error_call(call.clone(), "argument is not a character vector")),
    }
}

fn do_enc2native(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    enc2(call, args, "enc2native")
}

fn do_enc2utf8(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    enc2(call, args, "enc2utf8")
}

fn ieq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Encodings robigo can convert between: UTF-8 (= native) and ASCII.
#[derive(PartialEq, Clone, Copy)]
enum Enc {
    Utf8,
    Ascii,
    Latin1,
}

fn parse_enc(e: &str) -> Option<Enc> {
    if e.is_empty() || ieq(e, "UTF-8") || ieq(e, "UTF8") {
        Some(Enc::Utf8)
    } else if ieq(e, "ASCII") || ieq(e, "US-ASCII") || ieq(e, "ANSI_X3.4-1968") {
        Some(Enc::Ascii)
    } else if ieq(e, "latin1") || ieq(e, "ISO_8859-1") || ieq(e, "ISO-8859-1") || ieq(e, "ISO8859-1") {
        Some(Enc::Latin1)
    } else {
        None
    }
}

/// Whether the C library's iconv knows the conversion; R reports a failing
/// `iconv_open` as an unsupported conversion. "" is the native encoding.
fn system_iconv_supports(from: &str, to: &str) -> bool {
    use std::ffi::{CString, c_char, c_void};
    unsafe extern "C" {
        fn iconv_open(tocode: *const c_char, fromcode: *const c_char) -> *mut c_void;
        fn iconv_close(cd: *mut c_void) -> i32;
    }
    let native = |e: &str| if e.is_empty() { "UTF-8".to_string() } else { e.to_string() };
    let (Ok(f), Ok(t)) = (CString::new(native(from)), CString::new(native(to))) else {
        return false;
    };
    // SAFETY: both arguments are valid NUL-terminated strings; the
    // descriptor is closed right away.
    unsafe {
        let cd = iconv_open(t.as_ptr(), f.as_ptr());
        if cd as isize == -1 {
            return false;
        }
        iconv_close(cd);
    }
    true
}

// R: sysutils.c:do_iconv (UTF-8 and ASCII only)
fn do_iconv(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 6, "iconv")?;
    let x = &args[0].value;
    if x.is_null() {
        return Ok(Value::strs_from(["ASCII", "ISO-8859-1", "LATIN1", "UTF-8", "UTF8"]));
    }
    let one = |v: &Value, what: &str| -> RResult<RStr> {
        match v {
            Value::Str(s) if s.data.len() == 1 => Ok(s.data[0].clone()),
            _ => Err(Flow::error(format!("invalid '{}' argument", what))),
        }
    };
    let from = one(&args[1].value, "from")?.unwrap_or_else(|| Rc::from("NA"));
    let to = one(&args[2].value, "to")?.unwrap_or_else(|| Rc::from("NA"));
    let sub = one(&args[3].value, "sub")?;
    let mark = as_logical(&args[4].value);
    if mark == NA_LOGICAL {
        return Err(Flow::error("invalid 'mark' argument"));
    }
    let to_raw = as_logical(&args[5].value);
    if to_raw == NA_LOGICAL {
        return Err(Flow::error("invalid 'toRaw' argument"));
    }
    let (Some(fenc), Some(tenc)) = (parse_enc(&from), parse_enc(&to)) else {
        if !system_iconv_supports(&from, &to) {
            return Err(Flow::error(format!("unsupported conversion from '{}' to '{}'", from, to)));
        }
        return Err(Flow::error(format!("robigo: iconv() from '{}' to '{}' is not implemented yet", from, to)));
    };
    // Inputs: strings, or a list of raw vectors / NULL.
    let inputs: Vec<Option<Vec<u8>>> = match x {
        Value::Str(v) => v.data.iter().map(|e| e.as_ref().map(|s| s.as_bytes().to_vec())).collect(),
        Value::List(l) => {
            let mut v = Vec::new();
            for e in &l.data {
                match e {
                    Value::Null => v.push(None),
                    Value::Raw(r) => v.push(Some(r.data.clone())),
                    _ => {
                        return Err(Flow::error("'x' must be a character vector or a list of NULL or raw vectors"));
                    }
                }
            }
            v
        }
        _ => return Err(Flow::error("'x' must be a character vector")),
    };
    let mut results: Vec<Option<Vec<u8>>> = Vec::with_capacity(inputs.len());
    for inp in inputs {
        let Some(bytes) = inp else {
            results.push(None);
            continue;
        };
        // decode input to chars
        let chars: Vec<Result<char, u8>> = match fenc {
            Enc::Latin1 => bytes.iter().map(|&b| Ok(b as char)).collect(),
            Enc::Ascii => bytes.iter().map(|&b| if b < 0x80 { Ok(b as char) } else { Err(b) }).collect(),
            Enc::Utf8 => {
                let mut v = Vec::new();
                let mut rest: &[u8] = &bytes;
                while !rest.is_empty() {
                    match std::str::from_utf8(rest) {
                        Ok(s) => {
                            v.extend(s.chars().map(Ok));
                            rest = &[];
                        }
                        Err(e) => {
                            let good = e.valid_up_to();
                            let s = std::str::from_utf8(&rest[..good]).unwrap();
                            v.extend(s.chars().map(Ok));
                            v.push(Err(rest[good]));
                            rest = &rest[good + 1..];
                        }
                    }
                }
                v
            }
        };
        let mut out: Vec<u8> = Vec::new();
        let mut failed = false;
        for c in chars {
            let ok = match (c, tenc) {
                (Ok(c), Enc::Utf8) => {
                    let mut b = [0u8; 4];
                    out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
                    true
                }
                (Ok(c), Enc::Ascii) if c.is_ascii() => {
                    out.push(c as u8);
                    true
                }
                (Ok(c), Enc::Latin1) if (c as u32) < 0x100 => {
                    out.push(c as u32 as u8);
                    true
                }
                _ => false,
            };
            if ok {
                continue;
            }
            let Some(sub) = &sub else {
                failed = true;
                break;
            };
            match c {
                Ok(ch) if fenc == Enc::Utf8 && (&**sub == "Unicode" || &**sub == "c99") => {
                    let u = ch as u32;
                    let s = if &**sub == "Unicode" {
                        format!("<U+{:04X}>", u)
                    } else if u < 65536 {
                        format!("\\u{:04x}", u)
                    } else {
                        format!("\\U{:08x}", u)
                    };
                    out.extend_from_slice(s.as_bytes());
                }
                _ => {
                    let mut b = [0u8; 4];
                    let raw: Vec<u8> = match c {
                        Ok(ch) => match fenc {
                            Enc::Latin1 => vec![ch as u32 as u8],
                            _ => ch.encode_utf8(&mut b).as_bytes().to_vec(),
                        },
                        Err(byte) => vec![byte],
                    };
                    for byte in raw {
                        if &**sub == "byte" {
                            out.extend_from_slice(format!("<{:02x}>", byte).as_bytes());
                        } else {
                            out.extend_from_slice(sub.as_bytes());
                        }
                    }
                }
            }
        }
        results.push(if failed { None } else { Some(out) });
    }
    if to_raw != 0 {
        let items: Vec<Value> = results.into_iter().map(|r| r.map(Value::raws).unwrap_or(Value::Null)).collect();
        let mut ans = Value::list(items);
        if !matches!(x, Value::List(_)) {
            copy_all_attrs(x, &mut ans);
        } else {
            copy_all_attrs(x, &mut ans);
        }
        return Ok(ans);
    }
    let mut out = Vec::with_capacity(results.len());
    for r in results {
        match r {
            None => out.push(None),
            Some(bytes) => match tenc {
                Enc::Latin1 if bytes.iter().any(|&b| b >= 0x80) => {
                    return Err(Flow::error("robigo: latin1-encoded strings are not implemented yet"));
                }
                _ => match String::from_utf8(bytes) {
                    Ok(s) => out.push(Some(Rc::from(s.as_str()))),
                    Err(_) => return Err(Flow::error("robigo: strings that are not valid UTF-8 are not supported")),
                },
            },
        }
    }
    let mut ans = Value::strs(out);
    copy_all_attrs(x, &mut ans);
    Ok(ans)
}

// ----------------------------------------------------------------------
// utf8ToInt, intToUtf8, validUTF8, charToRaw, rawToChar

// R: raw.c:do_utf8ToInt
fn do_utf8toint(interp: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 1, "utf8ToInt")?;
    let x = match &args[0].value {
        Value::Str(v) if !v.data.is_empty() => v.clone(),
        _ => return Err(Flow::error("argument must be a character vector of length 1")),
    };
    if x.data.len() > 1 {
        interp
            .warning("argument should be a character vector of length 1\nall but the first element will be ignored")?;
    }
    match &x.data[0] {
        None => Ok(Value::int(NA_INTEGER)),
        Some(s) => Ok(Value::ints(s.chars().map(|c| c as i32).collect())),
    }
}

fn int_to_char(v: i32) -> Option<char> {
    if v < 0 || (0xD800..=0xDFFF).contains(&v) || v > 0x10FFFF { None } else { char::from_u32(v as u32) }
}

// R: raw.c:do_intToUtf8
fn do_inttoutf8(interp: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 3, "intToUtf8")?;
    let x = interp.coerce(&args[0].value, VType::Int)?;
    let Value::Int(xv) = &x else {
        return Err(Flow::error("argument 'x' must be an integer vector"));
    };
    let multiple = as_logical(&args[1].value);
    if multiple == NA_LOGICAL {
        return Err(Flow::error("argument 'multiple' must be TRUE or FALSE"));
    }
    let s_pair = as_logical(&args[2].value);
    if s_pair == NA_LOGICAL {
        return Err(Flow::error("argument 'allow_surrogate_pairs' must be TRUE or FALSE"));
    }
    if multiple != 0 {
        if s_pair != 0 {
            interp.warning("allow_surrogate_pairs = TRUE is incompatible with multiple = TRUE and will be ignored")?;
        }
        let out: Vec<RStr> = xv
            .data
            .iter()
            .map(|&v| {
                if v == NA_INTEGER {
                    None
                } else if v == 0 {
                    Some(Rc::from(""))
                } else {
                    int_to_char(v).map(|c| Rc::from(c.to_string().as_str()))
                }
            })
            .collect();
        return Ok(Value::strs(out));
    }
    let d = &xv.data;
    let mut out = String::new();
    let mut i = 0;
    while i < d.len() {
        let this = d[i];
        if this == NA_INTEGER || (0xDC00..=0xDFFF).contains(&this) || this > 0x10FFFF || this < 0 {
            return Ok(Value::str_na());
        }
        if (0xD800..=0xDBFF).contains(&this) {
            if s_pair == 0 || i + 1 >= d.len() {
                return Ok(Value::str_na());
            }
            let next = d[i + 1];
            if !(0xDC00..=0xDFFF).contains(&next) {
                return Ok(Value::str_na());
            }
            let cp = 0x10000 + (((this - 0xD800) as u32) << 10) + (next - 0xDC00) as u32;
            out.push(char::from_u32(cp).unwrap());
            i += 2;
            continue;
        }
        if this != 0 {
            out.push(char::from_u32(this as u32).unwrap());
        }
        i += 1;
    }
    Ok(Value::str(&out))
}

fn do_validutf8(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 1, "validUTF8")?;
    let Value::Str(x) = &args[0].value else {
        return Err(Flow::error("invalid 'x' argument"));
    };
    Ok(Value::lgls(vec![TRUE; x.data.len()]))
}

fn do_validenc(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 1, "validEnc")?;
    let Value::Str(x) = &args[0].value else {
        return Err(Flow::error("invalid 'x' argument"));
    };
    Ok(Value::lgls(vec![TRUE; x.data.len()]))
}

// R: raw.c:do_charToRaw
fn do_chartoraw(interp: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 1, "charToRaw")?;
    let x = match &args[0].value {
        Value::Str(v) if !v.data.is_empty() => v.clone(),
        _ => return Err(Flow::error("argument must be a character vector of length 1")),
    };
    if x.data.len() > 1 {
        interp
            .warning("argument should be a character vector of length 1\nall but the first element will be ignored")?;
    }
    let bytes = match &x.data[0] {
        None => b"NA".to_vec(),
        Some(s) => s.as_bytes().to_vec(),
    };
    Ok(Value::raws(bytes))
}

// R: raw.c:do_rawToChar
fn do_rawtochar(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 2, "rawToChar")?;
    let Value::Raw(x) = &args[0].value else {
        return Err(Flow::error("argument 'x' must be a raw vector"));
    };
    let multiple = as_logical(&args[1].value);
    if multiple == NA_LOGICAL {
        return Err(Flow::error("argument 'multiple' must be TRUE or FALSE"));
    }
    let to_str = |b: &[u8]| -> RResult<RStr> {
        if b.contains(&0) {
            return Err(Flow::error(format!(
                "embedded nul in string: '{}'",
                b.iter().map(|&c| if c == 0 { "\\0".to_string() } else { (c as char).to_string() }).collect::<String>()
            )));
        }
        match std::str::from_utf8(b) {
            Ok(s) => Ok(Some(Rc::from(s))),
            Err(_) => Err(Flow::error("robigo: strings that are not valid UTF-8 are not supported")),
        }
    };
    if multiple != 0 {
        let mut out = Vec::with_capacity(x.data.len());
        for &b in &x.data {
            out.push(if b == 0 { Some(Rc::from("")) } else { to_str(&[b])? });
        }
        Ok(Value::strs(out))
    } else {
        let end = x.data.iter().rposition(|&b| b != 0).map(|j| j + 1).unwrap_or(0);
        Ok(Value::strs(vec![to_str(&x.data[..end])?]))
    }
}

// ----------------------------------------------------------------------
// basename, dirname

/// R_ExpandFileName: `~` and `~/...` expand to the home directory.
pub fn expand_file_name(p: &str) -> String {
    if p == "~" || p.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            if !home.is_empty() {
                return format!("{}{}", home, &p[1..]);
            }
        }
    }
    p.to_string()
}

// R: util.c:do_basename
fn do_basename(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 1, "basename")?;
    let Value::Str(s) = &args[0].value else {
        return Err(Flow::error("a character vector argument expected"));
    };
    let mut out = Vec::with_capacity(s.data.len());
    for e in &s.data {
        match e {
            None => out.push(None),
            Some(p) => {
                let pp = expand_file_name(p);
                if pp.len() > 4095 {
                    return Err(Flow::error("path too long"));
                }
                let b = pp.as_bytes();
                let mut ll = b.len();
                while ll > 0 && b[ll - 1] == b'/' {
                    ll -= 1;
                }
                let mut ff = ll;
                while ff > 0 && b[ff - 1] != b'/' {
                    ff -= 1;
                }
                out.push(Some(Rc::from(&pp[ff..ll])));
            }
        }
    }
    Ok(Value::strs(out))
}

// R: util.c:do_dirname
fn do_dirname(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 1, "dirname")?;
    let Value::Str(s) = &args[0].value else {
        return Err(Flow::error("a character vector argument expected"));
    };
    let mut out = Vec::with_capacity(s.data.len());
    for e in &s.data {
        match e {
            None => out.push(None),
            Some(p) => {
                let pp = expand_file_name(p);
                if pp.len() > 4095 {
                    return Err(Flow::error("path too long"));
                }
                let b = pp.as_bytes();
                let mut ll = b.len();
                if ll == 0 {
                    out.push(Some(Rc::from("")));
                    continue;
                }
                while ll > 0 && b[ll - 1] == b'/' {
                    ll -= 1;
                }
                if ll == 0 {
                    out.push(Some(Rc::from("/")));
                    continue;
                }
                while ll > 0 && b[ll - 1] != b'/' {
                    ll -= 1;
                }
                if ll == 0 {
                    out.push(Some(Rc::from(".")));
                    continue;
                }
                while ll > 0 && b[ll - 1] == b'/' {
                    ll -= 1;
                }
                if ll == 0 {
                    out.push(Some(Rc::from("/")));
                    continue;
                }
                out.push(Some(Rc::from(&pp[..ll])));
            }
        }
    }
    Ok(Value::strs(out))
}

// R: platform.c:do_l10n_info (Linux, UTF-8 locale)
fn do_l10n_info(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::named_list(vec![
        ("MBCS", Value::lgl(true)),
        ("UTF-8", Value::lgl(true)),
        ("Latin-1", Value::lgl(false)),
        ("codeset", Value::str("UTF-8")),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths() {
        assert_eq!(ri18n_wcwidth('a'), 1);
        assert_eq!(ri18n_wcwidth('\u{4e2d}'), 2);
        assert_eq!(ri18n_wcwidth('\u{301}'), 0);
        assert_eq!(ri18n_wcwidth('\n'), 0);
    }

    #[test]
    fn strtoi() {
        assert_eq!(c_strtoi("ff", 16), Some(255));
        assert_eq!(c_strtoi("0x1A", 16), Some(26));
        assert_eq!(c_strtoi("077", 0), Some(63));
        assert_eq!(c_strtoi("12 ", 10), None);
        assert_eq!(c_strtoi(" -12", 10), Some(-12));
        assert_eq!(c_strtoi("z", 36), Some(35));
    }

    #[test]
    fn abbrev() {
        assert_eq!(stripchars("International", 4, true, false), "Intr");
        assert_eq!(stripchars("New Zealand", 4, true, false), "NwZl");
    }
}
