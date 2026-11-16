//! robigo-text's TRE against GNU R 4.6.2: `regexec()` of every pattern over
//! every text in `tests/data/regex-{,fuzz-,afuzz-}{utf8,c}.tsv`, recorded by
//! `oracle/regex.R` (which documents the format).
//!
//! `regexec` here is grep.c's `do_regexec` for one text: how it picks
//! bytes, wide or multibyte matching, what it compiles and how it reports
//! the result. The builtin itself lives in robigo-rtbuiltins; this copy
//! exists so the engine is checked on its own.

use robigo_text::tre::{self, Error, Match, Regex, Status};
use robigo_text::{Enc, Locale, Str, WideError, wtrans_char2};

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

fn pct(s: &[u8]) -> String {
    let mut out = String::new();
    for &k in s {
        if (0x20..=0x7d).contains(&k) && k != b'%' && k != b'|' {
            out.push(k as char);
        } else {
            out.push_str(&format!("%{k:02X}"));
        }
    }
    out
}

/// A CHARSXP: `None` is NA_STRING.
type RStr = Option<(Vec<u8>, Enc)>;

fn rstr(enc: &str, s: &str) -> RStr {
    let enc = match enc {
        "NA" => return None,
        "native" => Enc::Native,
        "UTF-8" => Enc::Utf8,
        "latin1" => Enc::Latin1,
        "bytes" => Enc::Bytes,
        e => panic!("unknown encoding {e}"),
    };
    Some((unpct(s), enc))
}

fn is_ascii(s: &RStr) -> bool {
    // NA_STRING is "NA"
    s.as_ref().is_none_or(|(b, _)| b.is_ascii())
}

fn has(s: &RStr, e: Enc) -> bool {
    s.as_ref().is_some_and(|(b, enc)| *enc == e && !b.is_ascii())
}

/// The UTF-8 locale's `mbcsValid`.
fn mbcs_valid(b: &[u8], loc: Locale) -> bool {
    !loc.mbcs() || {
        let mut i = 0;
        loop {
            match robigo_text::mbrtowc_utf8(b, i) {
                (0, _) => break true,
                (n, _) if n < 0 => break false,
                (n, _) => i += n as usize,
            }
        }
    }
}

enum Mode {
    Bytes,
    Wide,
    /// `tre_regcomp`/`tre_regexec`: the locale's multibyte encoding.
    Native,
}

/// What [`regexec`] returns where GNU R's TRE reads uninitialized memory,
/// so the recorded result is one of many.
const UNDEFINED: &str = "UNDEFINED";

/// `do_regexec(pattern, text)` for one text, in the result format of
/// `regex.R`.
// R: grep.c:do_regexec
fn regexec(pat: &RStr, text: &RStr, fixed: bool, icase: bool, use_bytes: bool, loc: Locale) -> String {
    let mut out = String::new();
    let warn = |out: &mut String, msg: &str| {
        out.push('W');
        out.push_str(&pct(msg.as_bytes()));
        out.push('|');
    };
    let mut cflags = tre::REG_EXTENDED;
    let mut icase = icase;
    if fixed && icase {
        warn(&mut out, "argument 'ignore.case = TRUE' will be ignored");
        icase = false;
    }
    if fixed {
        cflags |= tre::REG_LITERAL;
    }
    if icase {
        cflags |= tre::REG_ICASE;
    }
    let err = |mut out: String, msg: &str| {
        out.push('E');
        out.push_str(&pct(msg.as_bytes()));
        out
    };
    let p = pat.as_ref().expect("a pattern");

    let mut ub = use_bytes || p.1 == Enc::Bytes || has(text, Enc::Bytes);
    let itype = if ub { 'b' } else { 'c' };
    ub = ub || (p.0.is_ascii() && is_ascii(text));
    let mut use_wc = false;
    if !ub {
        use_wc = !fixed && loc.mbcs();
        use_wc = use_wc || has(pat, Enc::Utf8) || has(text, Enc::Utf8);
        // (not a Latin-1 locale)
        use_wc = use_wc || has(pat, Enc::Latin1) || has(text, Enc::Latin1);
    }
    let mode = if ub {
        Mode::Bytes
    } else if use_wc {
        Mode::Wide
    } else {
        Mode::Native
    };
    let wide = |s: &(Vec<u8>, Enc)| wtrans_char2(Str::new(&s.0, s.1), loc);

    let compiled = match mode {
        Mode::Bytes => Regex::new_bytes(&p.0, cflags, loc),
        Mode::Wide => match wide(p) {
            Ok(w) => Regex::new(&w, cflags, loc),
            Err(WideError::Invalid(m)) => {
                warn(&mut out, &format!("unable to translate '{m}' to a wide string"));
                return err(out, "regular expression is invalid");
            }
            Err(WideError::Bytes) => unreachable!("bytes are matched as bytes"),
        },
        Mode::Native => {
            if !mbcs_valid(&p.0, loc) {
                return err(out, "regular expression is invalid in this locale");
            }
            let w: Vec<i32> = if loc.mbcs() {
                wtrans_char2(Str::new(&p.0, Enc::Utf8), loc).expect("valid UTF-8")
            } else {
                p.0.iter().map(|&b| b as i32).collect()
            };
            Regex::new(&w, cflags, loc)
        }
    };
    let re = match compiled {
        Ok(re) => re,
        Err(Error::Status(s)) => return err(out, &format!("regcomp error: '{}'", s.message())),
        Err(e) => return err(out, &e.message()),
    };
    if re.undefined_in_c() {
        return UNDEFINED.to_string();
    }
    let Some(t) = text else {
        out.push_str("NA");
        return out;
    };
    let mut pm = vec![Match::UNSET; re.re_nsub() + 1];
    let rc = match mode {
        Mode::Bytes => re.exec_bytes(&t.0, &mut pm, 0),
        Mode::Wide => match wide(t) {
            Ok(w) => re.exec_wide(&w, &mut pm, 0),
            Err(WideError::Invalid(m)) => {
                warn(&mut out, &format!("unable to translate '{m}' to a wide string"));
                return err(out, "input string 1 is invalid in this locale");
            }
            Err(WideError::Bytes) => unreachable!("bytes are matched as bytes"),
        },
        Mode::Native => {
            if !mbcs_valid(&t.0, loc) {
                return err(out, "input string 1 is invalid in this locale");
            }
            if loc.mbcs() { re.exec_mbs(&t.0, &mut pm, 0) } else { re.exec_bytes(&t.0, &mut pm, 0) }
        }
    };
    let matched = match rc {
        Ok(m) => m,
        Err(Error::Status(Status::Espace)) => {
            warn(&mut out, "Out-of-memory error in regexp matching for element 1");
            false
        }
        Err(Error::Status(_)) => false,
        Err(e) => return err(out, &e.message()),
    };
    if ub {
        out.push(itype);
    }
    if matched {
        let v: Vec<String> = pm.iter().map(|m| format!("{}:{}", m.so + 1, m.eo - m.so)).collect();
        out.push_str(&v.join(","));
    } else {
        out.push('-');
    }
    out
}

fn check(file: &str, loc: Locale) {
    let path = format!("{}/tests/data/{file}", env!("CARGO_MANIFEST_DIR"));
    let data = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut texts: Vec<RStr> = Vec::new();
    let mut pats: Vec<RStr> = Vec::new();
    let (mut n, mut bad, mut undef) = (0, 0, 0);
    for line in data.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        match f[0] {
            "T" => texts.push(rstr(f[2], f[3])),
            "P" => pats.push(rstr(f[2], f[3])),
            "R" => {
                let pi: usize = f[1].parse().unwrap();
                let fl = f[2].as_bytes();
                let (fixed, icase, ub) = (fl[0] == b'1', fl[1] == b'1', fl[2] == b'1');
                let mut want = Vec::new();
                for r in &f[3..] {
                    match r.rsplit_once('~') {
                        Some((v, k)) => want.extend(std::iter::repeat_n(v, k.parse().unwrap())),
                        None => want.push(r),
                    }
                }
                assert_eq!(want.len(), texts.len(), "{file}: R {pi} {}", f[2]);
                let pat = &pats[pi - 1];
                for (ti, (t, w)) in texts.iter().zip(&want).enumerate() {
                    n += 1;
                    let got = std::panic::catch_unwind(|| regexec(pat, t, fixed, icase, ub, loc))
                        .unwrap_or_else(|_| "PANIC".to_string());
                    if got == UNDEFINED {
                        undef += 1;
                    } else if got != **w {
                        bad += 1;
                        if bad <= 40 {
                            let show = |s: &RStr| match s {
                                None => "NA".to_string(),
                                Some((b, e)) => format!("{:?} {}", e, pct(b)),
                            };
                            let ps = show(pat);
                            eprintln!(
                                "{file}: pattern {pi} [{}] flags {} text {} [{}]\n  want {w}\n  got  {got}",
                                if ps.len() > 80 { &ps[..80] } else { &ps },
                                f[2],
                                ti + 1,
                                show(t)
                            );
                        }
                    }
                }
            }
            k => panic!("unknown line kind {k}"),
        }
    }
    assert!(n > 10000, "{file}: only {n} cases");
    eprintln!("{file}: {n} cases, {undef} undefined in C");
    assert!(undef * 5 < n, "{file}: {undef} of {n} cases are undefined in C");
    assert_eq!(bad, 0, "{file}: {bad} of {n} cases differ");
}

#[test]
fn regexec_utf8() {
    check("regex-utf8.tsv", Locale::Utf8);
}

#[test]
fn regexec_c() {
    check("regex-c.tsv", Locale::C);
}

#[test]
fn regexec_fuzz_utf8() {
    check("regex-fuzz-utf8.tsv", Locale::Utf8);
}

#[test]
fn regexec_fuzz_c() {
    check("regex-fuzz-c.tsv", Locale::C);
}

#[test]
fn regexec_afuzz_utf8() {
    check("regex-afuzz-utf8.tsv", Locale::Utf8);
}

#[test]
fn regexec_afuzz_c() {
    check("regex-afuzz-c.tsv", Locale::C);
}
