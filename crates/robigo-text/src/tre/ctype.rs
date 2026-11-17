//! The character classes and case mappings TRE gets from the C library:
//! glibc's `wctype`/`iswctype` and `towupper`/`towlower` in the running
//! locale (tre-internal.h: `TRE_USE_SYSTEM_WCTYPE`, `tre_isctype`).
//!
//! C.UTF-8's tables are recorded from GNU R 4.6.2 (oracle/ctype.R). The C
//! locale classifies and maps ASCII only.

use super::tables::{CLASSES, TOLOWER, TOUPPER};
use crate::Locale;

/// A `wctype_t`: one of the classes glibc's `wctype()` knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Class {
    Alnum,
    Alpha,
    Blank,
    Cntrl,
    Digit,
    Graph,
    Lower,
    Print,
    Punct,
    Space,
    Upper,
    Xdigit,
    /// glibc's C.UTF-8 only.
    Combining,
    /// glibc's C.UTF-8 only.
    CombiningLevel3,
}

const NAMES: [(&str, Class); 14] = [
    ("alnum", Class::Alnum),
    ("alpha", Class::Alpha),
    ("blank", Class::Blank),
    ("cntrl", Class::Cntrl),
    ("digit", Class::Digit),
    ("graph", Class::Graph),
    ("lower", Class::Lower),
    ("print", Class::Print),
    ("punct", Class::Punct),
    ("space", Class::Space),
    ("upper", Class::Upper),
    ("xdigit", Class::Xdigit),
    ("combining", Class::Combining),
    ("combining_level3", Class::CombiningLevel3),
];

/// One bit per class for the ASCII range, so the common case is a lookup.
static ASCII: [u16; 128] = ascii_table();

const fn in_ranges(r: &[(u32, u32)], c: u32) -> bool {
    let mut i = 0;
    while i < r.len() {
        if r[i].0 <= c && c <= r[i].1 {
            return true;
        }
        i += 1;
    }
    false
}

const fn ascii_table() -> [u16; 128] {
    let mut t = [0u16; 128];
    let mut c = 0;
    while c < 128 {
        let mut k = 0;
        while k < CLASSES.len() {
            if in_ranges(CLASSES[k], c as u32) {
                t[c] |= 1 << k;
            }
            k += 1;
        }
        c += 1;
    }
    t
}

fn lookup(r: &[(u32, u32)], c: u32) -> bool {
    r.binary_search_by(|&(lo, hi)| {
        if hi < c {
            std::cmp::Ordering::Less
        } else if lo > c {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    })
    .is_ok()
}

fn map(t: &[(u32, u32)], c: u32) -> u32 {
    match t.binary_search_by_key(&c, |&(f, _)| f) {
        Ok(i) => t[i].1,
        Err(_) => c,
    }
}

impl Class {
    /// `wctype(name)` in `locale`; `None` is glibc's 0.
    pub fn from_name(name: &[i32], locale: Locale) -> Option<Class> {
        let found =
            NAMES.iter().find(|(n, _)| n.len() == name.len() && n.bytes().zip(name).all(|(a, &b)| a as i32 == b))?;
        match (found.1, locale) {
            (Class::Combining | Class::CombiningLevel3, Locale::C) => None,
            (k, _) => Some(k),
        }
    }

    /// `iswctype(c, class)` in `locale`.
    #[inline]
    pub fn contains(self, c: u32, locale: Locale) -> bool {
        if c < 128 {
            return ASCII[c as usize] & (1 << self as u16) != 0;
        }
        match locale {
            Locale::C => false,
            Locale::Utf8 => lookup(CLASSES[self as usize], c),
        }
    }
}

/// `iswspace(c)` in `locale` (also what datetime.c's strptime matches
/// white space with).
#[inline]
pub fn iswspace(c: u32, locale: Locale) -> bool {
    Class::Space.contains(c, locale)
}

/// `towupper(c)` in `locale`.
#[inline]
pub fn toupper(c: u32, locale: Locale) -> u32 {
    if c < 128 {
        return if c.wrapping_sub(b'a' as u32) < 26 { c - 32 } else { c };
    }
    match locale {
        Locale::C => c,
        Locale::Utf8 => map(&TOUPPER, c),
    }
}

/// `towlower(c)` in `locale`.
#[inline]
pub fn tolower(c: u32, locale: Locale) -> u32 {
    if c < 128 {
        return if c.wrapping_sub(b'A' as u32) < 26 { c + 32 } else { c };
    }
    match locale {
        Locale::C => c,
        Locale::Utf8 => map(&TOLOWER, c),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_and_beyond() {
        let u = Locale::Utf8;
        assert!(Class::Alpha.contains('a' as u32, u));
        assert!(Class::Alpha.contains(0xe9, u));
        assert!(!Class::Alpha.contains(0xe9, Locale::C));
        assert!(Class::Cntrl.contains(0, u));
        assert!(!Class::Print.contains(0, u));
        assert_eq!(toupper(0xe9, u), 0xc9);
        assert_eq!(toupper(0xe9, Locale::C), 0xe9);
        assert_eq!(tolower('Q' as u32, u), 'q' as u32);
        let name: Vec<i32> = "combining".bytes().map(i32::from).collect();
        assert_eq!(Class::from_name(&name, u), Some(Class::Combining));
        assert_eq!(Class::from_name(&name, Locale::C), None);
    }
}
