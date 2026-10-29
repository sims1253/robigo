//! `options()` (GNU R's options.c).
//!
//! Options live in `Interp::options` in insertion order, like R's
//! `.Options` pairlist; `options()` returns them sorted by name.

use std::cmp::Ordering;
use std::rc::Rc;

use super::internal;
use crate::coerce::{CoerceWarnings, as_integer, as_logical, to_int};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::s;
use crate::value::*;

/// R: coerce.c:asInteger — like `as_integer`, but issues the coercion
/// warnings `CoercionWarning()` does (out-of-integer-range reals,
/// non-numeric strings).
fn as_integer_warn(i: &mut Interp, v: &Value) -> RResult<i32> {
    let mut w = CoerceWarnings::default();
    let k = match v {
        Value::Raw(x) => x.data.first().map(|&b| b as i32).unwrap_or(NA_INTEGER),
        Value::Lgl(x) | Value::Int(x) => x.data.first().copied().unwrap_or(NA_INTEGER),
        v if v.is_atomic() && !v.is_empty() => to_int(v, &mut w)[0],
        _ => NA_INTEGER,
    };
    w.emit(i)?;
    Ok(k)
}

pub static DEFS: &[Builtin] = &[internal("options", do_options, -1).custom_vis()];

const R_MIN_WIDTH_OPT: i32 = 10;
const R_MAX_WIDTH_OPT: i32 = 10000;
const R_MIN_DIGITS_OPT: i32 = 1;
const R_MAX_DIGITS_OPT: i32 = 22;
const R_MIN_SCIPEN_OPT: i32 = -9;
const R_MAX_SCIPEN_OPT: i32 = 9999;
const R_MIN_EXPRESSIONS_OPT: i32 = 25;
const R_MAX_EXPRESSIONS_OPT: i32 = 500000;

/// The options GNU R sets at startup: `InitOptions()` in options.c and
/// the system profile (`Common.R`, `Rprofile.unix`) as run by Rscript.
pub fn init_options(interp: &mut Interp) {
    let lgl = Value::lgl;
    let int = Value::int;
    let st = Value::str;
    let opts: Vec<(&str, Value)> = vec![
        // R: options.c:InitOptions
        ("prompt", st("> ")),
        ("continue", st("+ ")),
        ("expressions", int(5000)),
        ("width", int(80)),
        ("deparse.cutoff", int(60)),
        ("digits", int(7)),
        ("echo", lgl(false)),
        ("verbose", lgl(false)),
        ("check.bounds", lgl(false)),
        ("keep.source", lgl(false)),
        ("keep.source.pkgs", lgl(false)),
        ("keep.parse.data", lgl(true)),
        ("keep.parse.data.pkgs", lgl(false)),
        ("warning.length", int(1000)),
        ("nwarnings", int(50)),
        ("OutDec", st(".")),
        ("CBoundsCheck", lgl(false)),
        ("matprod", st("default")),
        ("PCRE_study", lgl(false)),
        ("PCRE_use_JIT", lgl(true)),
        ("PCRE_limit_recursion", Value::lgl_na()),
        ("max.contour.segments", int(25000)),
        ("warnPartialMatchDollar", lgl(false)),
        ("warnPartialMatchArgs", lgl(false)),
        ("warnPartialMatchAttr", lgl(false)),
        ("showWarnCalls", lgl(false)),
        ("showErrorCalls", lgl(true)),
        ("showNCalls", int(50)),
        ("browserNLdisabled", lgl(false)),
        ("rl_word_breaks", st(" \t\n\"\\'`><=%;,|&{()}")),
        // R: library/profile/Common.R
        ("warn", int(0)),
        ("timeout", int(60)),
        ("encoding", st("native.enc")),
        ("show.error.messages", lgl(true)),
        ("scipen", int(0)),
        ("max.print", int(99999)),
        ("add.smooth", lgl(true)),
        ("catch.script.errors", lgl(false)),
        ("defaultPackages", default_packages()),
        // R: library/profile/Rprofile.unix
        ("papersize", st("a4")),
        ("printcmd", st(&std::env::var("R_PRINTCMD").unwrap_or_default())),
        ("dvipscmd", st(&std::env::var("DVIPS").unwrap_or_else(|_| "dvips".into()))),
        ("texi2dvi", st(&std::env::var("R_TEXI2DVICMD").unwrap_or_default())),
        ("browser", st(&std::env::var("R_BROWSER").unwrap_or_default())),
        ("pdfviewer", st(&std::env::var("R_PDFVIEWER").unwrap_or_default())),
        ("useFancyQuotes", lgl(true)),
    ];
    for (n, v) in opts {
        interp.set_option(n, v);
    }
}

fn default_packages() -> Value {
    match std::env::var("R_DEFAULT_PACKAGES") {
        Ok(dp) if dp == "NULL" => Value::strs(vec![]),
        Ok(dp) if !dp.is_empty() => Value::strs_from(dp.split(',').map(|s| s.trim())),
        _ => Value::strs_from(["datasets", "utils", "grDevices", "graphics", "stats", "methods"]),
    }
}

// ----------------------------------------------------------------------
// Collation

// ICU root collation order for ASCII (non-ignorable punctuation):
// whitespace, punctuation, symbols, digits, letters.
const PUNCT: &str = "\t\n\r _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";

const fn ascii_weight(c: u8) -> (u32, u32) {
    let p = PUNCT.as_bytes();
    let mut i = 0;
    while i < p.len() {
        if p[i] == c {
            return (1 + i as u32, 0);
        }
        i += 1;
    }
    if c.is_ascii_digit() {
        return (100 + (c - b'0') as u32, 0);
    }
    if c.is_ascii_alphabetic() {
        let l = c.to_ascii_lowercase();
        return (200 + (l - b'a') as u32, c.is_ascii_uppercase() as u32);
    }
    (1000 + c as u32, 0)
}

/// [`ascii_weight`] of every ASCII character, so that a comparison costs a
/// table load per character.
static ASCII_WEIGHTS: [(u32, u32); 128] = {
    let mut t = [(0, 0); 128];
    let mut c = 0;
    while c < 128 {
        t[c] = ascii_weight(c as u8);
        c += 1;
    }
    t
};

#[inline]
fn primary_weight(c: char) -> (u32, u32) {
    match ASCII_WEIGHTS.get(c as usize) {
        Some(&w) => w,
        None => (1000 + c as u32, 0),
    }
}

/// String collation as GNU R's `Scollate` does in a non-C locale (ICU
/// root collator): case-insensitive first, lower case before upper case
/// on ties. Exact for ASCII; other characters sort by code point.
///
/// This is the lexicographic order of the strings' primary weights, then of
/// their tertiary weights, then of their bytes, computed in one pass: the
/// first primary difference decides; otherwise (the weight sequences have
/// the strings' lengths) the shorter string, then the first tertiary
/// difference.
pub fn scollate(a: &str, b: &str) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    let (x, y) = (a.as_bytes(), b.as_bytes());
    let mut tert = Ordering::Equal;
    for (&p, &q) in x.iter().zip(y) {
        if p == q {
            continue;
        }
        if p >= 0x80 || q >= 0x80 {
            return scollate_chars(a, b);
        }
        let ((pp, pt), (qp, qt)) = (ASCII_WEIGHTS[p as usize], ASCII_WEIGHTS[q as usize]);
        if pp != qp {
            return pp.cmp(&qp);
        }
        if tert == Ordering::Equal {
            tert = pt.cmp(&qt);
        }
    }
    if x.len() != y.len() {
        // The shorter string is a prefix of the longer one up to ASCII
        // tertiary differences: the longer has more weights.
        return x.len().cmp(&y.len());
    }
    if tert != Ordering::Equal {
        return tert;
    }
    a.cmp(b)
}

/// [`scollate`] for strings that are not ASCII.
#[cold]
fn scollate_chars(a: &str, b: &str) -> Ordering {
    let prim = a.chars().map(|c| primary_weight(c).0).cmp(b.chars().map(|c| primary_weight(c).0));
    if prim != Ordering::Equal {
        return prim;
    }
    let tert = a.chars().map(|c| primary_weight(c).1).cmp(b.chars().map(|c| primary_weight(c).1));
    if tert != Ordering::Equal {
        return tert;
    }
    a.cmp(b)
}

// ----------------------------------------------------------------------

fn ok_int(v: i32) -> Value {
    Value::int(v)
}

fn invalid_value(name: &str) -> Flow {
    Flow::error(format!("invalid value for '{}'", name))
}

fn is_numeric(v: &Value) -> bool {
    matches!(v, Value::Int(_) | Value::Real(_) | Value::Lgl(_)) && !v.inherits("factor")
}

fn check_true_false(v: &Value, name: &str) -> RResult<()> {
    match v {
        Value::Lgl(l) if l.data.len() == 1 && l.data[0] != NA_LOGICAL => Ok(()),
        _ => Err(invalid_value(name)),
    }
}

fn int_from_real(x: f64) -> i32 {
    if x.is_nan() || x >= i32::MAX as f64 + 1.0 || x <= i32::MIN as f64 { NA_INTEGER } else { x as i32 }
}

/// R: options.c:FixupScipen
fn fixup_scipen(i: &mut Interp, v: &Value) -> RResult<i32> {
    if !is_numeric(v) || v.len() != 1 {
        return Err(Flow::error("invalid 'scipen'"));
    }
    let d = match v {
        Value::Real(r) => {
            let x = r.data[0];
            let d = int_from_real(x);
            if d == NA_INTEGER && !x.is_nan() {
                return Err(Flow::error(format!(
                    "setting scipen={} is out of range",
                    robigo_syntax::cfmt::fmt_g(x, 6, false, false)
                )));
            }
            d
        }
        _ => as_integer(v),
    };
    if d == NA_INTEGER || !(R_MIN_SCIPEN_OPT..=R_MAX_SCIPEN_OPT).contains(&d) {
        let dnew = if d == NA_INTEGER {
            0
        } else if d < R_MIN_SCIPEN_OPT {
            R_MIN_SCIPEN_OPT
        } else {
            R_MAX_SCIPEN_OPT
        };
        i.warning(format!("invalid 'scipen' {}, used {}", d, dnew))?;
        return Ok(dnew);
    }
    Ok(d)
}

fn string_opt(v: &Value, name: &str) -> RResult<Value> {
    let s = crate::coerce::as_string(v);
    match s {
        Some(s) if !s.is_empty() => Ok(Value::str(&s)),
        _ => Err(invalid_value(name)),
    }
}

/// Validate a new value for option `name` (R: options.c:do_options).
fn validate(i: &mut Interp, name: &str, argi: Value) -> RResult<Value> {
    Ok(match name {
        "width" => {
            let k = as_integer(&argi);
            if k == NA_INTEGER || !(R_MIN_WIDTH_OPT..=R_MAX_WIDTH_OPT).contains(&k) {
                return Err(Flow::error(format!(
                    "invalid '{}' parameter, allowed {}...{}",
                    name, R_MIN_WIDTH_OPT, R_MAX_WIDTH_OPT
                )));
            }
            ok_int(k)
        }
        "deparse.cutoff" => ok_int(as_integer(&argi)),
        "digits" => {
            let k = as_integer(&argi);
            if k == NA_INTEGER || !(R_MIN_DIGITS_OPT..=R_MAX_DIGITS_OPT).contains(&k) {
                return Err(Flow::error(format!(
                    "invalid '{}' parameter, allowed {}...{}",
                    name, R_MIN_DIGITS_OPT, R_MAX_DIGITS_OPT
                )));
            }
            ok_int(k)
        }
        "expressions" => {
            let k = as_integer(&argi);
            if k == NA_INTEGER || !(R_MIN_EXPRESSIONS_OPT..=R_MAX_EXPRESSIONS_OPT).contains(&k) {
                return Err(Flow::error(format!(
                    "invalid '{}' parameter, allowed {}...{}",
                    name, R_MIN_EXPRESSIONS_OPT, R_MAX_EXPRESSIONS_OPT
                )));
            }
            ok_int(k)
        }
        "keep.source" | "check.bounds" | "echo" | "verbose" => {
            if !matches!(argi, Value::Lgl(_)) || argi.len() != 1 {
                return Err(invalid_value(name));
            }
            Value::lgl_raw(as_logical(&argi))
        }
        "editor" if matches!(argi, Value::Str(_)) => string_opt(&argi, name)?,
        "continue" | "prompt" => string_opt(&argi, name)?,
        "contrasts" => {
            if !matches!(argi, Value::Str(_)) || argi.len() != 2 {
                return Err(invalid_value(name));
            }
            argi
        }
        "warn" => {
            if !is_numeric(&argi) || argi.len() != 1 {
                return Err(invalid_value(name));
            }
            let k = match &argi {
                Value::Real(r) => int_from_real(r.data[0]),
                _ => as_integer(&argi),
            };
            if k == NA_INTEGER {
                return Err(invalid_value(name));
            }
            ok_int(k)
        }
        "warning.length" => {
            let k = as_integer(&argi);
            if k == NA_INTEGER || !(100..=8170).contains(&k) {
                return Err(invalid_value(name));
            }
            argi
        }
        "warning.expression" => {
            if !argi.is_language() {
                return Err(invalid_value(name));
            }
            argi
        }
        "max.print" | "nwarnings" => {
            // R: options.c:do_options — asInteger() warns before the range
            // check (e.g. max.print = Inf).
            let k = as_integer_warn(i, &argi)?;
            if k == NA_INTEGER || k < 1 {
                return Err(invalid_value(name));
            }
            ok_int(k)
        }
        "scipen" => ok_int(fixup_scipen(i, &argi)?),
        "error" => {
            if argi.is_function() {
                Value::lang(argi, vec![])
            } else if !argi.is_language() {
                return Err(invalid_value(name));
            } else {
                argi
            }
        }
        "show.error.messages"
        | "catch.script.errors"
        | "warnPartialMatchDollar"
        | "warnPartialMatchArgs"
        | "warnPartialMatchAttr"
        | "showWarnCalls"
        | "showErrorCalls"
        | "browserNLdisabled"
        | "CBoundsCheck" => {
            check_true_false(&argi, name)?;
            argi
        }
        "OutDec" => {
            let ok = matches!(&argi, Value::Str(v) if v.data.len() == 1);
            if !ok {
                return Err(invalid_value(name));
            }
            let s = argi.str1().flatten().unwrap_or_else(|| Rc::from("NA"));
            if s.chars().count() != 1 {
                i.warning("'OutDec' must be a string of one character")?;
            }
            argi
        }
        "max.contour.segments" => {
            let k = as_integer(&argi);
            if k == NA_INTEGER || k < 0 {
                return Err(invalid_value(name));
            }
            ok_int(k)
        }
        "rl_word_breaks" => {
            if !matches!(&argi, Value::Str(v) if v.data.len() == 1) {
                return Err(invalid_value(name));
            }
            argi
        }
        "showNCalls" => {
            let k = as_integer(&argi);
            if k == NA_INTEGER || !(30..=500).contains(&k) || argi.len() != 1 {
                return Err(invalid_value(name));
            }
            ok_int(k)
        }
        "par.ask.default" => {
            return Err(Flow::error("\"par.ask.default\" has been replaced by \"device.ask.default\""));
        }
        "matprod" => {
            let s = crate::coerce::as_string(&argi);
            match s.as_deref() {
                // The Debian r-base-core build has OpenMP SIMD reduction, so
                // "default.simd" is accepted without the not-supported warning
                // (options.c guards it with HAVE_OPENMP_SIMDRED).
                Some("default") | Some("internal") | Some("blas") | Some("default.simd") => {}
                _ => return Err(invalid_value(name)),
            }
            argi
        }
        "PCRE_study" => {
            if let Value::Lgl(_) = argi {
                Value::lgl(as_logical(&argi) > 0)
            } else {
                let k = as_integer(&argi);
                let v = if k == NA_INTEGER || k < 0 { Value::lgl(false) } else { Value::int(k) };
                if !(k == NA_INTEGER || k < 0) {
                    i.warning("'PCRE_study' has no effect with PCRE2")?;
                }
                v
            }
        }
        "PCRE_use_JIT" => Value::lgl(as_logical(&argi) > 0),
        "PCRE_limit_recursion" => Value::lgl_raw(as_logical(&argi)),
        "stringsAsFactors" => {
            let l = as_logical(&argi);
            if !matches!(argi, Value::Lgl(_)) || argi.len() != 1 || l == NA_LOGICAL {
                return Err(invalid_value(name));
            }
            if l != 0 && !warned_strings_as_factors() {
                i.warning("'options(stringsAsFactors = TRUE)' is deprecated and will be disabled")?;
            }
            Value::lgl_raw(l)
        }
        _ => argi,
    })
}

const MANDATORY: &[&str] = &[
    "prompt",
    "continue",
    "expressions",
    "width",
    "deparse.cutoff",
    "digits",
    "echo",
    "verbose",
    "check.bounds",
    "keep.source",
    "keep.source.pkgs",
    "keep.parse.data",
    "keep.parse.data.pkgs",
    "warning.length",
    "nwarnings",
    "OutDec",
    "CBoundsCheck",
    "matprod",
    "PCRE_study",
    "PCRE_use_JIT",
    "PCRE_limit_recursion",
    "rl_word_breaks",
    "max.contour.segments",
    "warnPartialMatchDollar",
    "warnPartialMatchArgs",
    "warnPartialMatchAttr",
    "showWarnCalls",
    "showErrorCalls",
    "showNCalls",
    "browserNLdisabled",
    "warn",
    "max.print",
    "show.error.messages",
    "scipen",
];

/// Once-per-session flag for the stringsAsFactors deprecation warning;
/// returns whether it had already been set.
fn warned_strings_as_factors() -> bool {
    thread_local! {
        static WARNED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    WARNED.with(|w| w.replace(true))
}

/// R: options.c:do_options
fn do_options(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    if args.is_empty() {
        let mut items: Vec<(String, Value)> = i.options.clone();
        items.sort_by(|a, b| scollate(&a.0, &b.0));
        let names = Value::strs_from(items.iter().map(|(n, _)| n.as_str()));
        let mut v = Value::list(items.into_iter().map(|(_, v)| v).collect());
        v.set_attr_raw(s::NAMES, names);
        i.visible = true;
        return Ok(v);
    }
    // A single unnamed list (or pairlist) argument supplies the options.
    let mut items: Vec<(Option<RStr>, Value)> = Vec::new();
    let single_list = args.len() == 1
        && args[0].tag.is_none()
        && matches!(args[0].value, Value::List(_) | Value::Pairlist(_) | Value::Null | Value::Expr(_));
    if single_list {
        match &args[0].value {
            Value::Null => {}
            Value::Pairlist(p) => {
                for a in &p.items {
                    items.push((Some(Some(a.tag.map(|t| t.name()).unwrap_or_else(|| Rc::from("")))), a.value.clone()));
                }
            }
            v @ (Value::List(l) | Value::Expr(l)) => {
                let n = l.data.len();
                if n > 0 {
                    let names = match v.get_attr(s::NAMES) {
                        Some(Value::Str(nm)) if nm.data.len() == n => nm.data.clone(),
                        _ => return Err(Flow::error("list argument has no valid names")),
                    };
                    for (k, x) in l.data.iter().enumerate() {
                        items.push((Some(names[k].clone()), x.clone()));
                    }
                }
            }
            _ => unreachable!(),
        }
    } else {
        for a in &args {
            items.push((Some(Some(a.tag.map(|t| t.name()).unwrap_or_else(|| Rc::from("")))), a.value.clone()));
        }
    }
    let n = items.len();
    let mut values: Vec<Value> = Vec::with_capacity(n);
    let mut names: Vec<RStr> = Vec::with_capacity(n);
    let mut visible = false;
    for (namei, argi) in items {
        let namei: Rc<str> = namei.flatten().unwrap_or_else(|| Rc::from("NA"));
        if !namei.is_empty() {
            names.push(Some(namei.clone()));
            if argi.is_null() {
                if MANDATORY.contains(&&*namei) {
                    return Err(Flow::error(format!("option '{}' cannot be deleted", namei)));
                }
                values.push(i.set_option(&namei, Value::Null));
            } else {
                let v = validate(i, &namei, argi)?;
                values.push(i.set_option(&namei, v));
            }
        } else {
            let tag = match &argi {
                Value::Str(v) if !v.data.is_empty() => v.data[0].clone(),
                _ => return Err(Flow::error("invalid argument")),
            };
            let tag: Rc<str> = tag.unwrap_or_else(|| Rc::from("NA"));
            if &*tag == "par.ask.default" {
                return Err(Flow::error("\"par.ask.default\" has been replaced by \"device.ask.default\""));
            }
            values.push(i.get_option(&tag).unwrap_or(Value::Null));
            names.push(Some(tag));
            visible = true;
        }
    }
    let mut v = Value::list(values);
    v.set_attr_raw(s::NAMES, Value::strs(names));
    i.visible = visible;
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one-pass comparison agrees with the definition (weights of whole
    /// strings compared in turn) on every pair of short strings.
    #[test]
    fn scollate_matches_weight_sequences() {
        let alphabet =
            ["a", "A", "b", "B", "z", "1", "9", " ", "_", "-", ".", "\t", "\u{1}", "\u{7f}", "é", "É", "€", "ü"];
        let mut strs = vec![String::new()];
        for _ in 0..3 {
            let prev: Vec<String> = strs.clone();
            for s in &prev {
                for c in alphabet {
                    strs.push(format!("{s}{c}"));
                }
            }
        }
        strs.sort();
        strs.dedup();
        for a in &strs {
            for b in &strs {
                assert_eq!(scollate(a, b), scollate_chars(a, b), "{a:?} vs {b:?}");
            }
        }
    }
}
