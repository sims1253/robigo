## 2026-11-16: string collation through ICU4X, ICU 76.1's root collator

- What: the foreman's collation package (B1's decision, in
  m1-foreman.md). robigo-rt's string orderings (`<`, `>`, `<=`, `>=`),
  `min`/`max`, `pmin`/`pmax` and `options()` with no arguments raised "not
  implemented yet" under ICU collation. They now collate as GNU R 4.6.2
  does in the oracle's `C.UTF-8`, with ICU's root collator, through ICU4X:
  no libicu, no port of ICU.
- Tree: branch `g3/collation` on main `dd932c78`, then merged with main
  `3e5e4aa8` (#693, #695). Binaries: `cargo build
  --release --locked -p robigo-rtapi` (`robigo-rt`), main's and the
  branch's, each in a private target directory. Machine: AMD Ryzen 9 5900X
  (24 threads), WSL2 Linux 6.18, rustc 1.100.0, valgrind 3.27.1. GNU R:
  the 4.6.2 oracle image (`scripts/oracle-run.sh 4.6.2`) for every
  recording and probe.

### The oracle's collator

- The image is Debian trixie with `libicu76` 76.1-4: `extSoftVersion()`
  says ICU "76.1", `capabilities("ICU")` is TRUE, and after the first
  comparison `icuGetCollate()` is "root". ICU 76.1 is CLDR 46 and Unicode
  16.0.
- util.c's `Scollate` decides at its first call. C collation (`strcoll` of
  the native translations) applies when `LC_ALL` (or, unset or empty,
  `LC_COLLATE`) is "C" in the environment, or when `getLocale()` is "C"
  (`R_ICU_LOCALE` when set and not empty, else `setlocale(LC_COLLATE)`).
  Otherwise GNU R calls `uloc_setDefault(getLocale())` and
  `ucol_open(NULL)`, and sets no attribute: tertiary strength,
  non-ignorable, no case level, normalization off. It then compares the
  `translateCharUTF8` of both strings with `ucol_strcollIter`.
- robigo-rtbuiltins' `text::scollate` ports that decision. The isolate's
  `Locale::collate_c` is `setlocale(LC_COLLATE)`. The environment is read
  once a process, as GNU R reads it (robigo-rt has no `Sys.setlocale`).
  An `R_ICU_LOCALE` other than "C" names a locale robigo has no data for,
  and is "not implemented yet".

### The data

ICU4X publishes no data for ICU 76.1. `icu_collator_data`'s sources by
release: 1.5.x ICU 75 (CLDR 45, 46-BETA2); 2.0.0-beta1 `release-76-1` and
CLDR 46.0.0 (but beta code); 2.0.0 ICU 77; 2.1 to 2.3 ICU 78.1rc (CLDR 48);
2.4 Unicode 18. So the data is generated: `scripts/collate-data.sh` runs
ICU4X's datagen at the version of the runtime crates, 2.3.0. That is the
last minor version whose datagen reads ICU's `icuexport`, and the workspace
pins `icu_collator ~2.3.1`. Its sources are pinned by sha256:

- the collation markers come from `icuexportdata_release-76-1.zip`, with
  CLDR 46.0.0 for datagen's locale data;
- the NFD markers (the collator reads its input through them, and robigo's
  FCD check reads them) come from `icuexportdata_release-77-1.zip`. 77.1
  is also Unicode 16.0; 76.1's export lacks a field this datagen needs
  (`cap`). `robigo-collate-data`'s `nfd_is_unicode_16` checks every code
  point's decomposition and combining class against Python 3.14's
  `unicodedata` 16.0.0 (14,183 nontrivial ones, hashed).
  `one_step_decompositions_reach_nfd` checks that the one-step
  decompositions the FCD check reads end where NFD ends.

Only the root collation is generated: root, jamo, special primaries,
metadata, diacritics; no tailoring and no reordering. Datagen does not
filter the metadata and diacritics markers by locale (about 1.3 KB). Han
characters are in radical-stroke order (`--collation-root-han unihan`),
because ICU's root orders them so. Datagen's default, implicit weights in
code point order, failed the order test at position 1,520: GNU R says
`"\u7e37" > "\U000218a2"`, a URO ideograph after an Extension B one.

The data is baked Rust (`crates/robigo-collate-data/data`, ICU4X's
"compiled data"), behind a provider of the markers the root collator
reads. The generated code's `unsafe` (payloads over unchecked bytes) is in
one module that names its allowance (`docs/architecture.md`, "Unsafe
code"). The data is ICU's and CLDR's, so the crate carries the Unicode
License v3 (`LICENSE-Unicode`) beside robigo's GPL.

### Normalization: FCD only

ICU4X reads its input through NFD and has no option to turn that off. ICU,
with normalization off, reads the input as it is. The two agree on text in
FCD form (UTN #5; NFC and NFD text are FCD) and not on other text. Two
cases where GNU R 4.6.2 orders canonically equivalent strings, which ICU4X
calls equal:

- `"a\u0301\u0323" < "a\u0323\u0301"` is TRUE (a mark of class 220 after
  one of 230, bare or in a precomposed letter);
- `"\u0f71\u0f81" < "\u0f71\u0f71\u0f80"` is TRUE. ICU's collation never
  reads the Tibetan composite vowels U+0F73, U+0F75 and U+0F81 as FCD
  (collationfcd.h: Tibetan contractions start inside them). The order
  test failed on such strings until robigo refused them.

`robigo_text::collate::root_strcoll` therefore orders strings that are FCD
in ICU's sense and raises "not implemented yet" for others. An FCD string
has no code point whose decomposition starts with a nonzero class and
either follows a greater trailing class or ends in another class. A string
is equal to itself, whatever its form. The check reads the combining
classes of each code point's one-step decompositions. Code points below
U+0300 (lead bytes below 0xCC) skip it.

### Proof

- `crates/robigo-text/tests/collate.rs` (`root_strcoll_matches_gnu_r`)
  generates 50,000 strings from a fixed seed. They are FCD by
  construction and cover ASCII, controls, Latin with precomposed and
  combining accents, Greek, Cyrillic, Hebrew, Arabic, Devanagari, Thai,
  Tibetan, Hangul syllables and jamo, kana, CJK (URO, Extension B,
  compatibility), digits of four scripts, symbols, emoji, private use,
  unassigned code points and noncharacters. The test checks robigo against
  GNU R's `order(x, method = "shell")` of the first 10,000 and the sign of
  `Scollate` for 20,000 pairs: a string and one small edit of it (case,
  accent, an ignorable, a deletion, a swap, a canonically equivalent
  spelling). Of the pairs, 6,974 are `=`, 6,999 `<` and 6,027 `>`. GNU R's
  answers are in `tests/data/collate.tsv` (69 KB), recorded by
  `oracle/record.sh` (`oracle/collate.R`, in the oracle image). Both
  checks pass, in debug and release builds.
- robigo-rtbuiltins' oracle, recorded on 4.6.2 (`tests/oracle/record.sh`):
  - `relop.R` gains ICU orderings over every pairing of case, accents,
    digits, punctuation, empty strings, NA, numbers, ten scripts, and
    UTF-8, latin1, native, invalid native and "bytes" strings (`I.*`,
    1,596 relop cases in all);
  - `collate.R` covers min/max/pmin/pmax of strings with the same marks
    (418 cases).

  All match, also with a collection at each call out. The not-FCD cases
  are the only gaps listed.
- `tests/rt/strings/collate.R`, recorded on 4.6.2 and ratcheted, checks
  the R level:
  - the orderings, a rank by comparison of 85 strings built from pieces of
    each kind, and the scripts;
  - invalid native bytes against U+FFFD;
  - `min`/`max` with NA and `na.rm`, `pmin`;
  - the order of `options()`'s names.
- `min`/`max` lost a string's encoding mark: they returned a latin1
  string's bytes as native. They now keep the CHARSXP's bytes and mark, as
  GNU R keeps the CHARSXP (`collate.R`'s `max.l1` cases).
- `scripts/rt-ci.py` passes every step on `70f222e7`: conformance 249/254
  plain and under each collector mode and the small vector limit (248/253
  under `ROBIGO_NO_INPLACE`). The five failures predate the branch and have
  no ratchet marker. `scripts/lint.sh` is clean.

### Size

The `robigo-rt` binary, release (the branch at `70f222e7`, on `dd932c78`):

| bytes | main `dd932c78` | branch | change |
|---|---|---|---|
| file | 15,838,912 | 17,198,064 | +1,359,152 |
| stripped | 3,164,768 | 3,841,432 | +676,664 (+21.4%) |
| `.rodata` | 168,872 | 676,456 | +507,584 |
| `.text` | 2,430,154 | 2,567,050 | +136,896 |
| `.eh_frame` | 161,688 | 170,184 | +8,496 |
| `.data.rel.ro` | 66,424 | 68,664 | +2,240 |

`.rodata` is the data; most of it is the root collation's trie with Han
in radical-stroke order. Implicit Han order would be about 330 KB less,
but it is not ICU's order. `.text` is ICU4X's collator and normalizer.

### Speed

A comparison-bound script, 1.6M comparisons of strings with accents,
Cyrillic, CJK and Hangul (`vapply(x, function(t) sum(x < t), 0)`, 400
strings, ten times):

- wall time, three runs each: robigo-rt 0.252 to 0.268 s (an empty script
  0.006 s), GNU R 4.6.2 in the image 0.140 to 0.143 s (empty 0.027 s);
- callgrind (one tenth of the script): ICU4X's comparison takes about
  1,550 instructions a pair, robigo's FCD check about 135 a string. A
  first FCD check that ran the normalizer on each code point cost 420.

ICU4C has a fast path for Latin text that ICU4X lacks. A faster path here
would mean porting collation logic, which this package rules out: a
follow-up for the foreman.

### Upstream

`icu_collator` 2.3.1's debug assertion that a decomposing nonstarter
(U+0344, ...) was decomposed before reordering fails on `"\u0f57\u0344"`
against `"\u0f57\u0344x"`. It fails with ICU4X's own compiled data too,
and 2.4.0 keeps the assertion. Release code orders those strings as GNU R
does: 144 comparisons of FCD strings with U+0340, U+0341, U+0343 or
U+0344 after nine kinds of base all match. So the workspace builds `icu_collator` without debug
assertions in the dev profile (`Cargo.toml`). This is not reported
upstream yet.

### Not in this package

- `sort`, `order`, `rank` and `xtfrm` are not in the base subset (none is
  in `base-subset.txt`). `base-bound.txt` binds `sort`, `order` and
  `sort.list` statically, but robigo-rt implements none of their
  internals (`order`, `radixsort`, `sorted_fpass`, `sort.default`'s and
  `rank`'s are missing for every type). The large random sort is
  therefore checked at the text level and, at the R level, as a rank by
  comparison.
- Text not in FCD form, an `R_ICU_LOCALE` other than "C", and
  `icuSetCollate`/`icuGetCollate` (not bound) are "not implemented yet".
- `options()`'s list: robigo-rt sets fewer options than GNU R's startup
  (`browser`, `useFancyQuotes`, ...). The order of the names it has
  matches.
