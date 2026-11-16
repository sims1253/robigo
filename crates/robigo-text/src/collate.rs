//! GNU R's `Scollate` where its ICU collator is the root one (the oracle's
//! `C.UTF-8`; util.c:Scollate): `ucol_strcollIter` of two UTF-8 strings
//! with ICU's default attributes (tertiary strength, non-ignorable, no case
//! level, normalization off), through ICU4X's collator over ICU 76.1's root
//! data (`robigo-collate-data`).
//!
//! ICU4X reads its input through NFD and cannot turn that off. ICU with
//! normalization off reads it as it is, which orders text in FCD form (UTN
//! #5; NFC and NFD text are FCD) as NFD does, and other text otherwise: GNU
//! R 4.6.2 says `"ạ́" < "ạ́"`, two canonically
//! equivalent strings ICU4X calls equal. So [`root_strcoll`] orders FCD
//! strings only.

use std::cmp::Ordering;
use std::sync::LazyLock;

use icu_collator::options::CollatorOptions;
use icu_collator::{Collator, CollatorBorrowed};
use icu_normalizer::properties::{CanonicalCombiningClassMap, CanonicalCombiningClassMapBorrowed};
use icu_normalizer::{DecomposingNormalizer, DecomposingNormalizerBorrowed};

/// A string [`root_strcoll`] cannot order as ICU does: it is not in FCD form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotFcd;

/// The root collator and the NFD data its FCD check reads, built once.
struct Root {
    collator: CollatorBorrowed<'static>,
    nfd: DecomposingNormalizerBorrowed<'static>,
    ccc: CanonicalCombiningClassMapBorrowed<'static>,
}

static ROOT: LazyLock<Root> = LazyLock::new(|| {
    let data = robigo_collate_data::Root;
    // the owned values hold references to the baked data; leaked once, they
    // lend 'static borrows
    let collator = Collator::try_new_unstable(&data, Default::default(), CollatorOptions::default())
        .expect("robigo-collate-data holds the root collation");
    let nfd = DecomposingNormalizer::try_new_nfd_unstable(&data).expect("robigo-collate-data holds NFD");
    let ccc = CanonicalCombiningClassMap::try_new_unstable(&data).expect("robigo-collate-data holds NFD");
    Root {
        collator: Box::leak(Box::new(collator)).as_borrowed(),
        nfd: Box::leak(Box::new(nfd)).as_borrowed(),
        ccc: Box::leak(Box::new(ccc)).as_borrowed(),
    }
});

/// `ucol_strcollIter(root, a, b)` for UTF-8 `a` and `b`, ill-formed
/// sequences read as U+FFFD (as ICU's UTF-8 iterator and ICU4X read them),
/// or [`NotFcd`] when either is not in FCD form.
// R: util.c:Scollate
pub fn root_strcoll(a: &[u8], b: &[u8]) -> Result<Ordering, NotFcd> {
    // ICU compares a string equal to itself, whatever its form
    if a == b {
        return Ok(Ordering::Equal);
    }
    let r = &*ROOT;
    if !r.is_fcd(a) || !r.is_fcd(b) {
        return Err(NotFcd);
    }
    Ok(r.collator.compare_utf8(a, b))
}

impl Root {
    /// `s` is in FCD form: no code point whose decomposition starts with a
    /// nonzero combining class follows one whose decomposition ends with a
    /// greater one. An ill-formed sequence is U+FFFD, a starter.
    fn is_fcd(&self, s: &[u8]) -> bool {
        // below U+0300 (lead bytes below 0xCC) every decomposition starts
        // with a starter
        if s.iter().all(|&b| b < 0xCC) {
            return true;
        }
        let mut trail = 0;
        for chunk in s.utf8_chunks() {
            for c in chunk.valid().chars() {
                let (lead, t) = self.lead_trail_ccc(c);
                if lead != 0 && trail > lead {
                    return false;
                }
                trail = t;
            }
            if !chunk.invalid().is_empty() {
                trail = 0;
            }
        }
        true
    }

    /// The combining classes of the first and the last code point of `c`'s
    /// canonical decomposition.
    fn lead_trail_ccc(&self, c: char) -> (u8, u8) {
        if (c as u32) < 0xC0 {
            return (0, 0);
        }
        let mut buf = [0; 4];
        let s = c.encode_utf8(&mut buf);
        if self.nfd.is_normalized(s) {
            let k = self.ccc.get32_u8(c as u32);
            return (k, k);
        }
        let d = self.nfd.normalize(s);
        let mut cs = d.chars();
        let first = cs.next().unwrap_or(c);
        let last = cs.next_back().unwrap_or(first);
        (self.ccc.get32_u8(first as u32), self.ccc.get32_u8(last as u32))
    }
}

#[cfg(test)]
mod tests {
    use super::{NotFcd, ROOT, root_strcoll};
    use std::cmp::Ordering::{Equal, Greater, Less};

    #[test]
    fn orders_as_the_root_collator() {
        let s = |a: &str, b: &str| root_strcoll(a.as_bytes(), b.as_bytes());
        assert_eq!(s("a", "B"), Ok(Less));
        assert_eq!(s("B", "b"), Ok(Greater));
        assert_eq!(s("", "a"), Ok(Less));
        assert_eq!(s("e\u{301}", "\u{e9}"), Ok(Equal));
        assert_eq!(s("\u{1100}\u{1161}", "\u{ac00}"), Ok(Equal));
        assert_eq!(root_strcoll(b"\xff\xfe", "\u{fffd}\u{fffd}".as_bytes()), Ok(Equal));
        assert_eq!(root_strcoll(b"\xe2\x82", "\u{fffd}".as_bytes()), Ok(Equal));
    }

    #[test]
    fn refuses_what_is_not_fcd() {
        let s = |a: &str, b: &str| root_strcoll(a.as_bytes(), b.as_bytes());
        // a mark of class 220 after one of 230, bare or in a precomposed letter
        assert_eq!(s("a\u{301}\u{323}", "a\u{323}\u{301}"), Err(NotFcd));
        assert_eq!(s("a\u{323}\u{301}", "\u{e1}\u{323}"), Err(NotFcd));
        // ... but the same string is equal to itself
        assert_eq!(s("a\u{301}\u{323}", "a\u{301}\u{323}"), Ok(Equal));
        // FCD: classes that do not decrease, and U+1EA1's trailing 220 before 230
        assert_eq!(s("\u{1ea1}\u{301}", "a\u{323}\u{301}"), Ok(Equal));
        // U+0F73 decomposes to classes 129 then 130
        let r = &*ROOT;
        assert!(r.is_fcd("\u{f73}\u{f72}".as_bytes()));
        assert!(!r.is_fcd("\u{f73}\u{f71}".as_bytes()));
        assert!(r.is_fcd(b"\xcc\xff\x81"));
    }
}
