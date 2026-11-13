//! robigo-text against GNU R 4.6.2: the cases in `tests/data`, recorded by
//! `oracle/record.sh` (see `oracle/cases.R` for how each is produced).
//!
//! Fields are tab-separated; byte strings are percent-encoded (`%XX`),
//! doubles are C99 hex floats or NA/NaN/Inf/-Inf.
//!
//! - `R digits scipen nsmall width dec xs w,d,e s...`: `format_real` of
//!   `xs`, then `encode_real0` of each in `max(w, width)`.
//! - `I width xs w s...`, `L width xs w s...`: integers and logicals.
//! - `D x s`: `as.character(x)`, 15 digits through `encode_real_drop0`.
//! - `S enc bytes quote justify width rstrlen0 rstrlenq out`:
//!   `encodeString()` (strings marked UTF-8 kept in UTF-8).
//! - `P enc bytes quote out`: `print()` of one string (native encoding).
//! - `N quote justify width out`: `encodeString(NA_character_)`.

use robigo_text::num::{
    RealFormat, encode_integer, encode_logical, encode_real_drop0, encode_real0, format_integer, format_logical,
    format_real,
};
use robigo_text::string::{Justify, encode_string, format_string, rstrlen};
use robigo_text::{Enc, Locale, NA_INTEGER, NA_REAL, PrintParams, Str};

fn unpct(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).expect("a %XX escape"));
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

/// A C99 hex float (`sprintf("%a")`), or R's names for the non-finite.
fn hexf(s: &str) -> f64 {
    match s {
        "NA" => return NA_REAL,
        "NaN" => return f64::NAN,
        "Inf" => return f64::INFINITY,
        "-Inf" => return f64::NEG_INFINITY,
        _ => {}
    }
    let (neg, s) = match s.strip_prefix('-') {
        Some(t) => (true, t),
        None => (false, s),
    };
    let s = s.strip_prefix("0x").expect("0x");
    let (mant, exp) = s.split_once('p').expect("p");
    let mut exp: i32 = exp.parse().expect("an exponent");
    let mut m: u64 = 0;
    let (int, frac) = mant.split_once('.').unwrap_or((mant, ""));
    for c in int.chars().chain(frac.chars()) {
        m = m * 16 + c.to_digit(16).expect("a hex digit") as u64;
    }
    exp -= 4 * frac.len() as i32;
    assert!(m < 1 << 53, "{s}: more than 53 bits");
    // (m < 2^53 is exact; scale by powers of two that stay normal)
    let mut x = m as f64;
    while exp < -1000 {
        x *= 2f64.powi(-1000);
        exp += 1000;
    }
    while exp > 1000 {
        x *= 2f64.powi(1000);
        exp -= 1000;
    }
    x *= 2f64.powi(exp);
    if neg { -x } else { x }
}

fn enc_of(s: &str) -> Enc {
    match s {
        "native" => Enc::Native,
        "UTF-8" => Enc::Utf8,
        "latin1" => Enc::Latin1,
        "bytes" => Enc::Bytes,
        _ => panic!("unknown encoding {s}"),
    }
}

fn quote_of(s: &str) -> u8 {
    unpct(s).first().copied().unwrap_or(0)
}

fn show(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Runs every case of `file` and reports all failures at once.
fn check(file: &str, data: &str, locale: Locale) {
    let pp0 = PrintParams { locale, ..PrintParams::DEFAULT };
    let mut fails = Vec::new();
    let mut n = 0;
    for (ln, line) in data.lines().enumerate() {
        let f: Vec<&str> = line.split('\t').collect();
        let at = format!("{file}:{}", ln + 1);
        let mut fail = |what: String| fails.push(format!("{at}: {what}\n    {line}"));
        n += 1;
        match f[0] {
            "R" => {
                let digits: i32 = f[1].parse().unwrap();
                let pp = PrintParams { digits, scipen: f[2].parse().unwrap(), ..pp0 };
                let nsmall: i32 = f[3].parse().unwrap();
                let width: i32 = f[4].parse().unwrap();
                let dec = unpct(f[5]);
                let xs: Vec<f64> = f[6].split(',').filter(|v| !v.is_empty()).map(hexf).collect();
                let wde: Vec<i32> = f[7].split(',').map(|v| v.parse().unwrap()).collect();
                let want = RealFormat { w: wde[0], d: wde[1], e: wde[2] };
                let got = format_real(&xs, nsmall, &pp);
                if got != want {
                    fail(format!("format_real: got {got:?}, want {want:?}"));
                    continue;
                }
                let fmt = RealFormat { w: want.w.max(width), ..want };
                for (x, s) in xs.iter().zip(&f[8..]) {
                    let mut out = Vec::new();
                    encode_real0(&mut out, *x, fmt, &dec, &pp);
                    if out != unpct(s) {
                        fail(format!("encode_real0({x:e}): got {:?}, want {:?}", show(&out), show(&unpct(s))));
                    }
                }
            }
            "I" | "L" => {
                let width: i32 = f[1].parse().unwrap();
                let xs: Vec<i32> = f[2]
                    .split(',')
                    .filter(|v| !v.is_empty())
                    .map(|v| match v {
                        "NA" => NA_INTEGER,
                        "T" => 1,
                        "F" => 0,
                        v => v.parse().unwrap(),
                    })
                    .collect();
                let want: i32 = f[3].parse().unwrap();
                let got = if f[0] == "I" { format_integer(&xs, &pp0) } else { format_logical(&xs, &pp0) };
                if got != want {
                    fail(format!("format: got {got}, want {want}"));
                    continue;
                }
                for (x, s) in xs.iter().zip(&f[4..]) {
                    let mut out = Vec::new();
                    if f[0] == "I" {
                        encode_integer(&mut out, *x, want.max(width), &pp0);
                    } else {
                        encode_logical(&mut out, *x, want.max(width), &pp0);
                    }
                    if out != unpct(s) {
                        fail(format!("encode({x}): got {:?}, want {:?}", show(&out), show(&unpct(s))));
                    }
                }
            }
            "D" => {
                let x = hexf(f[1]);
                let pp = PrintParams { digits: 15, ..pp0 };
                let fmt = format_real(&[x], 0, &pp);
                let mut out = Vec::new();
                encode_real_drop0(&mut out, x, fmt, b".", &pp);
                if out != unpct(f[2]) {
                    fail(format!("as.character: got {:?}", show(&out)));
                }
            }
            "S" => {
                let bytes = unpct(f[2]);
                let s = Str::new(&bytes, enc_of(f[1]));
                let quote = quote_of(f[3]);
                let justify = Justify::from_r(f[4].parse().unwrap()).unwrap();
                let r0: Option<i32> = f[6].parse().ok();
                let rq: i32 = f[7].parse().unwrap();
                if let Some(r0) = r0 {
                    let got = rstrlen(s, 0, locale);
                    if got != r0 {
                        fail(format!("rstrlen(s, 0): got {got}, want {r0}"));
                    }
                }
                let got = rstrlen(s, quote, locale);
                if got != rq {
                    fail(format!("rstrlen(s, quote): got {got}, want {rq}"));
                }
                // encodeString's own width: the element's, with quotes
                let w = match f[5] {
                    "NA" => format_string([Some(s)], quote, &pp0).max(0),
                    w => w.parse().unwrap(),
                };
                let w = if justify == Justify::None { 0 } else { w };
                let mut out = Vec::new();
                encode_string(&mut out, Some(s), w, quote, justify, true, &pp0);
                if out != unpct(f[8]) {
                    fail(format!("encode_string: got {:?}, want {:?}", show(&out), show(&unpct(f[8]))));
                }
            }
            "P" => {
                let bytes = unpct(f[2]);
                let s = Str::new(&bytes, enc_of(f[1]));
                let quote = quote_of(f[3]);
                let w = format_string([Some(s)], quote, &pp0);
                let mut out = Vec::new();
                encode_string(&mut out, Some(s), w, quote, Justify::Left, false, &pp0);
                if out != unpct(f[4]) {
                    fail(format!("print: got {:?}, want {:?}", show(&out), show(&unpct(f[4]))));
                }
            }
            "N" => {
                let quote = quote_of(f[1]);
                let justify = Justify::from_r(f[2].parse().unwrap()).unwrap();
                let w = match f[3] {
                    // Rstrlen(NA_STRING, quote) is that of "NA", plus quotes
                    "NA" => 2 + if quote != 0 { 2 } else { 0 },
                    w => w.parse().unwrap(),
                };
                let w = if justify == Justify::None { 0 } else { w };
                let mut out = Vec::new();
                encode_string(&mut out, None, w, quote, justify, false, &pp0);
                if out != unpct(f[4]) {
                    fail(format!("encode_string(NA): got {:?}", show(&out)));
                }
            }
            k => panic!("{at}: unknown case kind {k}"),
        }
    }
    assert!(n > 100, "{file}: only {n} cases");
    assert!(fails.is_empty(), "{} of {n} cases fail:\n{}", fails.len(), fails.join("\n"));
}

#[test]
fn numbers() {
    check("numbers.tsv", include_str!("data/numbers.tsv"), Locale::Utf8);
}

#[test]
fn strings_utf8() {
    check("strings-utf8.tsv", include_str!("data/strings-utf8.tsv"), Locale::Utf8);
}

#[test]
fn strings_c() {
    check("strings-c.tsv", include_str!("data/strings-c.tsv"), Locale::C);
}
