//! `robigo_text::collate::root_strcoll` against GNU R 4.6.2's `Scollate`
//! in the oracle image (ICU 76.1's root collator, `C.UTF-8`), on strings
//! this file generates (`strings`, from a fixed seed): GNU R's
//! `order(x, method = "shell")` of the first `ORDER`, and the sign of
//! `Scollate(a, b)` (`(a > b) - (a < b)`) for the `PAIRS` pairs after them,
//! recorded in `tests/data/collate.tsv` by `oracle/record.sh` (which runs
//! this test with `ROBIGO_COLLATE_STRINGS` set to write the strings, then
//! `oracle/collate.R` over them).
//!
//! The strings are in FCD form by construction (combining marks follow
//! bases without a trailing mark, in classes that never decrease) and
//! cover ASCII, controls, Latin with precomposed and combining accents,
//! Greek, Cyrillic, Hebrew, Arabic, Devanagari, Thai, Tibetan, Hangul
//! (syllables and jamo), kana, CJK, digits of several scripts, symbols,
//! emoji, private use, unassigned code points and noncharacters. The pairs
//! are a string and one small edit of it (case, accent, an ignorable, a
//! deletion, a swap, a canonically equivalent spelling, ...), so their
//! order is mostly decided past the primary level.

use robigo_text::collate::root_strcoll;
use std::cmp::Ordering;

const ORDER: usize = 10_000;
const PAIRS: usize = 20_000;

/// splitmix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len())]
    }
}

/// Ranges of starters.
const BASES: &[(u32, u32)] = &[
    (0x20, 0x7e),
    (0x20, 0x7e),
    (0x20, 0x7e),
    (0x41, 0x5a),
    (0x61, 0x7a),
    (0x61, 0x7a),
    (0x30, 0x39),
    (0x01, 0x08),
    (0x0b, 0x0c),
    (0x0e, 0x1f),
    (0x7f, 0x7f),
    (0x85, 0x85),
    (0xa0, 0xbf),
    (0xdf, 0xdf),
    (0xe6, 0xe6),
    (0xf8, 0xf8),
    (0x111, 0x111),
    (0x142, 0x142),
    (0x131, 0x131),
    (0x14b, 0x14b),
    (0xfe, 0xfe),
    (0xf0, 0xf0),
    (0x391, 0x3a9),
    (0x3b1, 0x3c9),
    (0x410, 0x44f),
    (0x5d0, 0x5ea),
    (0x627, 0x64a),
    (0x660, 0x669),
    (0x915, 0x939),
    (0x93e, 0x94c),
    (0x966, 0x96f),
    (0xe01, 0xe2e),
    (0xe40, 0xe44),
    (0xf40, 0xf69),
    (0xf90, 0xfb9),
    (0x1100, 0x1112),
    (0x1161, 0x1175),
    (0x11a8, 0x11c2),
    (0xac00, 0xd7a3),
    (0xac00, 0xd7a3),
    (0x3041, 0x3096),
    (0x30a1, 0x30fa),
    (0x4e00, 0x9fff),
    (0x4e00, 0x9fff),
    (0x20000, 0x2a6df),
    (0xf900, 0xfa6d),
    (0x2000, 0x200d),
    (0x2010, 0x2027),
    (0x2028, 0x2029),
    (0x20ac, 0x20ac),
    (0x2122, 0x2122),
    (0x2460, 0x2473),
    (0x221e, 0x221e),
    (0x2713, 0x2713),
    (0x2665, 0x2665),
    (0x1f600, 0x1f64f),
    (0x1f1e6, 0x1f1ff),
    (0xfeff, 0xfeff),
    (0xfffd, 0xfffd),
    (0xff01, 0xff5e),
    (0xe000, 0xe0ff),
    (0x378, 0x379),
    (0x380, 0x383),
    (0x2fe0, 0x2fef),
    (0xe0080, 0xe00ff),
    (0xe0001, 0xe0001),
    (0xe0020, 0xe007f),
    (0xfdd0, 0xfdef),
    (0xfffe, 0xffff),
    (0x10fffe, 0x10ffff),
    (0xad, 0xad),
];

/// Precomposed letters (their decompositions end in a mark).
const PRECOMPOSED: &[u32] = &[
    0xe9, 0xe8, 0xea, 0xeb, 0xc9, 0xf1, 0xd1, 0xfc, 0xdc, 0xf6, 0xe5, 0xc5, 0xe7, 0x151, 0x105, 0x117, 0x1d6, 0x1ea1,
    0x1ebf, 0x1ee3, 0x1ed, 0x1e09, 0x22b, 0x3ac, 0x3ce, 0x451, 0x439, 0x419, 0x3070, 0x30d1, 0x304c, 0x958, 0xfb2a,
    0x1e0d, 0x1e63, 0x1e69, 0x1fb3, 0x1f80,
];

/// Combining marks (U+0344 decomposes; the Tibetan composite vowels, which
/// ICU's collation never reads as FCD, are left out).
const MARKS: &[u32] = &[
    0x300, 0x301, 0x302, 0x303, 0x304, 0x306, 0x307, 0x308, 0x30a, 0x30c, 0x20d7, 0x323, 0x331, 0x327, 0x328, 0x31b,
    0x345, 0x1dce, 0x302a, 0x5b0, 0x5bc, 0x5c1, 0x64b, 0x650, 0x652, 0x93c, 0x94d, 0xe48, 0xf71, 0xf72, 0xf74, 0xf80,
    0x3099, 0x344,
];

fn ch(c: u32) -> char {
    char::from_u32(c).expect("a scalar value")
}

/// The classes of the first and the last code point of `c`'s canonical
/// decomposition (ICU4X's NFD data, robigo-collate-data's).
fn lead_trail(c: char) -> (u8, u8) {
    use icu_normalizer::DecomposingNormalizer;
    use icu_normalizer::properties::CanonicalCombiningClassMap;
    use std::sync::LazyLock;
    static NFD: LazyLock<(DecomposingNormalizer, CanonicalCombiningClassMap)> = LazyLock::new(|| {
        let data = robigo_collate_data::Root;
        (
            DecomposingNormalizer::try_new_nfd_unstable(&data).unwrap(),
            CanonicalCombiningClassMap::try_new_unstable(&data).unwrap(),
        )
    });
    let (nfd, ccc) = (NFD.0.as_borrowed(), NFD.1.as_borrowed());
    let d = nfd.normalize(c.encode_utf8(&mut [0; 4])).into_owned();
    let k = |x: Option<char>| ccc.get32_u8(x.unwrap() as u32);
    (k(d.chars().next()), k(d.chars().next_back()))
}

/// A cluster: a base (none, only first) and marks after it, each whose
/// first class is at least the last class before it: FCD.
fn cluster(r: &mut Rng, first: bool) -> String {
    let mut s = String::new();
    match r.below(10) {
        0 | 1 => s.push(ch(r.pick(PRECOMPOSED))),
        2 if first => {}
        _ => {
            let (lo, hi) = r.pick(BASES);
            s.push(ch(lo + r.below((hi - lo + 1) as usize) as u32));
        }
    }
    if s.is_empty() || r.below(4) == 0 {
        let mut trail = s.chars().next().map_or(0, |c| lead_trail(c).1);
        let mut ms: Vec<(char, (u8, u8))> = (0..1 + r.below(3))
            .map(|_| {
                let c = ch(r.pick(MARKS));
                (c, lead_trail(c))
            })
            .collect();
        ms.sort_by_key(|m| m.1.0);
        for (c, (lead, t)) in ms {
            if lead >= trail && (lead == 0 || lead == t) {
                s.push(c);
                trail = t;
            }
        }
    }
    s
}

fn string(r: &mut Rng) -> Vec<String> {
    let n = [0, 1, 1, 2, 2, 3, 3, 4, 5, 6, 8, 12][r.below(12)];
    (0..n).map(|i| cluster(r, i == 0)).collect()
}

/// `s` is FCD to ICU's collation, by the generator's own reading of the
/// classes (an edit that breaks it is not made).
fn fcd(s: &str) -> bool {
    let mut trail = 0;
    s.chars().all(|c| {
        let (lead, t) = lead_trail(c);
        let ok = lead == 0 || (lead >= trail && lead == t);
        trail = t;
        ok
    })
}

/// One small edit of `s`, cluster-wise; `s` again where the edit would
/// leave FCD (marks before a leading run of bare marks).
fn edit(r: &mut Rng, s: &[String]) -> Vec<String> {
    let t = edit1(r, s);
    if fcd(&t.concat()) { t } else { s.to_vec() }
}

fn edit1(r: &mut Rng, s: &[String]) -> Vec<String> {
    let mut t = s.to_vec();
    let i = if t.is_empty() { 0 } else { r.below(t.len()) };
    let accent: &[(char, &str, &str)] = &[
        ('a', "\u{e1}", "a\u{301}"),
        ('e', "\u{e9}", "e\u{301}"),
        ('e', "\u{e8}", "e\u{300}"),
        ('o', "\u{f6}", "o\u{308}"),
        ('u', "\u{fc}", "u\u{308}"),
        ('n', "\u{f1}", "n\u{303}"),
        ('c', "\u{e7}", "c\u{327}"),
        ('A', "\u{c5}", "A\u{30a}"),
    ];
    match r.below(12) {
        _ if t.is_empty() => t.push(cluster(r, true)),
        0 | 1 => {
            // case
            let c = &t[i];
            let f: String =
                c.chars()
                    .map(|x| {
                        if x.is_lowercase() {
                            x.to_uppercase().next().unwrap()
                        } else {
                            x.to_lowercase().next().unwrap()
                        }
                    })
                    .collect();
            if f.chars().count() == c.chars().count() {
                t[i] = f;
            }
        }
        2 => {
            let (base, pre, dec) = r.pick(accent);
            t[i] = [base.to_string(), pre.to_string(), dec.to_string()][r.below(3)].clone();
        }
        3 => {
            // a precomposed letter as its canonical decomposition, or the
            // reverse
            let (_, pre, dec) = r.pick(accent);
            if t[i] == pre {
                t[i] = dec.to_string();
            } else {
                t.insert(i, [pre, dec][r.below(2)].to_string());
            }
        }
        4 => t.insert(i, r.pick(&["\u{1}", "\u{200b}", "\u{ad}", "\u{feff}", "\u{7f}", "\u{e0041}"]).to_string()),
        5 => {
            t.remove(i);
        }
        6 if t.len() > 1 => {
            let j = if i + 1 < t.len() { i + 1 } else { i - 1 };
            t.swap(i, j);
        }
        7 => t.push(cluster(r, false)),
        8 => {
            let c = t[i].chars().next().unwrap_or('a');
            if c.is_ascii_graphic() {
                t[i] = ch(c as u32 - 0x21 + 0xff01).to_string();
            }
        }
        9 => t[i] = r.pick(&[" ", "\u{a0}", "-", "\u{2010}", "_", "1", "\u{661}", "\u{ff11}"]).to_string(),
        10 => {
            // the same text again (equal, or decided by an earlier
            // difference)
        }
        _ => t[i] = cluster(r, i == 0),
    }
    t
}

/// The strings `collate.R` orders and compares: `ORDER`, then `PAIRS`
/// pairs.
fn strings() -> Vec<String> {
    let mut r = Rng(0x726f_6269_676f_2d63);
    let mut out: Vec<String> = (0..ORDER).map(|_| string(&mut r).concat()).collect();
    for _ in 0..PAIRS {
        let s = string(&mut r);
        let t = edit(&mut r, &s);
        out.push(s.concat());
        out.push(t.concat());
    }
    out
}

#[test]
fn root_strcoll_matches_gnu_r() {
    let xs = strings();
    if let Some(p) = std::env::var_os("ROBIGO_COLLATE_STRINGS") {
        let mut f = format!("{ORDER} {PAIRS}\n");
        for x in &xs {
            assert!(!x.contains(['\n', '\r']), "a line break in {x:?}");
            f.push_str(x);
            f.push('\n');
        }
        std::fs::write(p, f).unwrap();
        return;
    }
    let data = include_str!("data/collate.tsv");
    let field = |tag: &str| -> &str {
        data.lines().find_map(|l| l.strip_prefix(tag)).unwrap_or_else(|| panic!("collate.tsv: no {tag:?} line"))
    };
    // the strings GNU R read: their count and bytes
    let bytes: usize = xs.iter().map(|x| x.len()).sum();
    assert_eq!(field("N\t"), format!("{ORDER}\t{PAIRS}\t{}\t{bytes}", xs.len()), "collate.tsv is of other strings");
    let cmp = |a: &str, b: &str| {
        root_strcoll(a.as_bytes(), b.as_bytes()).unwrap_or_else(|_| panic!("{a:?} or {b:?} is not FCD"))
    };
    // order(x, method = "shell"): Scollate, ties in index order
    let want: Vec<usize> = field("O\t").split(',').map(|i| i.parse::<usize>().unwrap() - 1).collect();
    let mut got: Vec<usize> = (0..ORDER).collect();
    got.sort_by(|&i, &j| cmp(&xs[i], &xs[j]).then(i.cmp(&j)));
    if let Some(k) = (0..ORDER).find(|&k| got[k] != want[k]) {
        panic!("order differs at {k}: robigo {:?}, GNU R {:?}", xs[got[k]], xs[want[k]]);
    }
    let signs = field("P\t").as_bytes();
    assert_eq!(signs.len(), PAIRS);
    let mut bad = Vec::new();
    for (k, &s) in signs.iter().enumerate() {
        let (a, b) = (&xs[ORDER + 2 * k], &xs[ORDER + 2 * k + 1]);
        let want = match s {
            b'<' => Ordering::Less,
            b'=' => Ordering::Equal,
            _ => Ordering::Greater,
        };
        if cmp(a, b) != want {
            bad.push(format!("{a:?} vs {b:?}: GNU R {want:?}"));
        }
    }
    assert!(bad.is_empty(), "{} of {PAIRS} pairs differ:\n{}", bad.len(), bad[..bad.len().min(40)].join("\n"));
}
