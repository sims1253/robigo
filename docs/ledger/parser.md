# parser: robigo-syntax

Metric: parse-tree agreement with GNU R 4.5.0's `parse()` over the parse
corpora (`scripts/parse-corpus.py check`), then parse throughput (MB/s,
callgrind, over the whole corpus).

Corpora state (GNU R 4.5.0 = Debian r-base-core 4.5.0-3, the CI oracle):

| corpus | sources | match |
|---|---|---|
| fastr | 31374 | 31374 |
| gnur-examples | 1153 | 1153 |
| rlib (GNU R src/library) | 834 | 834 |
| rtests (GNU R tests) | 95 | 95 |
| gnur-tests | 59 | 59 |

## 2026-10-27: resumed the WIP branch; numeric literals byte-exact; 100% on all five corpora

- why: the track's job is byte-exact parse trees over a large corpus.
  The saved WIP (origin/track/parser-corpus) was reviewed piece by
  piece, verified, corrected and finished; rlib was 833/834 with a
  Latin-1 known divergence.
- what (kept):
  - strings as raw bytes (`Const::Str(Vec<u8>)`): `\xNN`/octal escapes
    keep their single byte; Hershey.R (Latin-1 `\xc1`) now matches, the
    known divergence is gone (834/834).
  - the WIP's `NumericValue` scan port and the placeholder token were
    kept after checking them against gram.y.
- what (corrected, each found by the corpus and pinned by tests/parse):
  - `y$NULL` was accepted (`$` with NULL_CONST): R's grammar is
    `expr '$' SYMBOL | expr '$' STR_CONST` only — reverted to
    "unexpected 'NULL'".
  - `function('a'=1)` was accepted: formals are SYMBOL only →
    "unexpected string constant".
  - `f(''=1)`, `''(x)`, ``` `` ``` (empty backtick): `install("")`
    errors "attempt to use zero-length variable name" — the backtick
    case in the lexer (SymbolValue installs there), the tag/function
    case at the grammar action; recorded as position-less errors
    ("ERROR 0 0"), which is what GNU R's `parse()` reports.
  - leading `;` (`;1`, `1;;2`, `repeat ; 1`): a `;` where an expression
    is expected is the unexpected token itself (gram.y's `prog` has no
    leading-`;` rule; `{ ; 1 }` stays legal — `exprlist` skips them).
    This is one of the two parser gaps the cli track's echo.rs works
    around; the other (`ends_in_comment`) is still open (below).
  - `<<` without `-`: the lexer's ERROR token → "unexpected input".
  - error precedence: GNU R interleaves lexing with parsing, so
    `list(''' 'a')` reports the earlier grammar error ("unexpected
    string constant") and not the INCOMPLETE_STRING the eager lexer
    hits later. The first lexical error now becomes an `Invalid` token
    the parser reports when it reaches it.
- numbers (`util.c:R_strtod5` port, `hex_to_f64`/`decimal_to_f64`):
  - hexadecimal: an x87-accurate emulation. Key discoveries, each
    verified against compiled C and R: the 64-bit significand
    saturates — later hex digits are below half an ulp and are dropped
    without rounding (`0x...fe6` then `a` keeps the mantissa); the
    scale-by-2^k goes through a *double* p2 that overflows to inf at
    2^1024, so `0x1p-1074` underflows to 0 and `0x1p-1075` is 0 while
    `0x1p-1023` is exact; `ans == 0` skips scaling (`0x0p100000` is 0);
    one final rounding to f64.
  - decimal: the same 80-bit emulation (LD, round-to-nearest-even per
    op, gradual underflow, `expn < -307` divide-fac branch, the
    `expn + ndigits < -300` prescale, digit-at-a-time accumulation
    where the digit is lost once the ulp exceeds 2). Needed because
    Rust's correctly-rounded parse differs from R's double rounding on
    17-digit literals (3 fastr cases, e.g. `0.76077021908142894`).
  - verified: 7038 random hex + 14027 random decimal literals (extreme
    exponents, 400-digit monsters, all scaling branches) byte-identical
    with GNU R's `%.17g` dumps, plus the curated set in
    tests/parse/literals.R.
- harness: `scripts/parse-corpus.py` (record/check/show/fetch-cran),
  `robigo parse-batch`, `scripts/oracle-parse-batch.R`; committed
  dumps for the three in-repo corpora; ratchets in tests/parse/passing.
- result: fastr 31297/31374 (as saved) -> 31374/31374; rlib 833/834 ->
  834/834; rtests 94/95 -> 95/95; gnur-tests 59/59; gnur-examples
  1153/1153. Kept.
- rejected on the way (all measured against R):
  - "hex mantissa rounds per digit like an integer accumulator" —
    wrong twice: the digit's weight is the *units place of the value*,
    not the mantissa's low nibble, and a *double* p2 overflows at
    2^1024 (found via C traces with %La).
  - "Rust's correctly-rounded parse is close enough for decimals" —
    3 corpus mismatches say no.
  - ld_div quotient rounding to a fixed 2^63 grid — the grid is 2^63
    or 2^64 depending on whether the quotient is below 1 (`0.466714940227e-233`).

## Open

- CRAN corpus: `scripts/parse-corpus.py fetch-cran` + record + check;
  expect new mismatch classes (encodings, test-only syntax).
- Throughput: parse MB/s over the corpus with callgrind (parse-batch is
  the tool); the LD decimal path costs ~10 LD ops per literal — measure
  before optimizing, then consider a provably-equivalent fast path for
  short literals.
- The cli track's second workaround (PR #6): `ends_in_comment`
  (gram.y's SkipComment sets EndOfFile = 2, promoting statuses to
  PARSE_INCOMPLETE) belongs in robigo-syntax as a `parse_console`
  entry point (R's R_Parse1Buffer with gend = 0). Also from their
  ledger: multi-byte/BOM error contexts work in bytes (R's
  R_ParseContext ring); robigo's token spans should end on character
  boundaries; invalid UTF-8 should error "invalid multibyte character
  in parser" (robigo's lossy conversion accepts it today).
- srcrefs: not dumped (keep.source = FALSE); if the dump ever includes
  them, byte positions follow xxbyteno.

## 2026-10-28: parse-throughput baseline (main 5b0b240, harness committed)

- why: the track's second metric; `scripts/parse-perf.py wall|cg` over
  all six corpora (41,067 sources, 51.3 MB), `parse-batch --parse-only`
  with a checksum that folds constant values, name and string bytes.
- baseline (best of 3; callgrind single runs, deterministic):
  wall 25.60 MB/s total (rlib 28.6, rtests 22.3, fastr 9.9,
  gnur-tests 23.2, gnur-examples 25.7, cran 29.3);
  callgrind 202,567 instr/kB total (rlib 203.7k, rtests 246.8k,
  fastr 358.8k, gnur-tests 249.3k, gnur-examples 187.6k, cran 186.4k).
- callgrind profile (rlib, 1.54G instr): allocator paths ~35%
  (_int_malloc 11%, free 7%, malloc 4.6%, _int_free_chunk 3.2%,
  malloc_consolidate 3.2%, memcpy_avx 5.7% — much of it Vec/token
  copying), tokenize 10.6%, Tok::clone 2.6%, Tok/Token drop glue 3%,
  Expr drop glue 2.3%, `String::from_utf8_lossy(...).into_owned()`
  (a full copy of every source) 3.2%, contains_placeholder 1%,
  the LD number paths only ~1.3%. Every Sym/Special token and every
  escape-free string allocates; the parser clones whole Tokens
  (String included) to peek.
- so the plan is: tokens and string literals borrow the source, names
  intern as `Rc<str>`, no Token clones in the parser, zero-copy UTF-8
  in the harness, a placeholder-seen flag, and only then a
  provably-equivalent short-literal number fast path.
- corpus ratchet holds at 100% on all six corpora on the baseline
  build; note for this machine: the shared `~/.cache/cargo-target`
  bled another worktree's stale `librobigo_syntax` rlib into bin
  builds (lib compiles, bin "cannot find dump_error") — use a private
  `CARGO_TARGET_DIR` for parser-track builds.

## 2026-10-27 (later): CRAN corpus at 100% (7552/7552)

- why: CRAN sources are the largest and most varied slice of the metric.
- what:
  - `fetch-cran`: 148 packages downloaded as source tarballs
    (`download.packages(type = "source")`, mirror set explicitly because
    `--vanilla` leaves `options("repos")` empty), 7552 `.R` sources.
  - tag/symbol-name escaping in the dump format (`encodeString(quote="")`
    on both sides): string tags like `"\n"=1` and backtick symbols can
    hold raw newlines, which broke the one-line-per-expression format.
  - `parse(file=)` reads through a text connection whose newline
    translation turns CR and CRLF into LF before parsing; parse-batch
    now does the same (gram.y itself never sees CR from a file).
  - parse-batch mis-parsed the manifest's enc column when a sha column
    followed, so Latin-1 sources went through lossy UTF-8 (found as
    µ → U+FFFD; a misleading "non-determinism" chase — it was
    deterministic, my probe manifests just lacked the sha column).
  - glibc C.UTF-8 `iswprint` parity in `encode_string`: a 735-range
    printable table generated by scanning `encodeString` over every
    scalar value (U+2028/9, U+0E00, U+2072 print as `\uXXXX`;
    `\U{%06x}` above the BMP) — `// R: printutils.c`.
  - `%prec LOW`: bodies of `function`/`if`/`for`/`while`/`repeat` end
    before `?` (`function() 1 ? 2` is `(function() 1) ? 2`); our
    Pratt parser parses them at `BP_LOW = BP_QUESTION + 1`.
- result: cran 0/7552 recorded -> 7552/7552; the other five corpora stay
  at 100%; ratchet in tests/parse/passing/cran.txt. Kept.
