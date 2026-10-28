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

## 2026-10-28: parser diagnostics and bounded text parsing

Baseline: origin/main `49e0ee5`, 147/191 passes with no recorded-pass
regressions, using the parser filter below and GNU R 4.5.0's R_HOME.
The previous model's saved 146/191 result predates the parser-speed merge.
PR #34 is merged; this work continues on `track/parser-diagnostics`.

Failure groups on that baseline, classified from each failing script:

- 11 parse-data/source metadata cases. Port `RecordParents`, parse-data
  records and source attributes before implementing `getParseData` output.
- 15 imported scripts contain literal `<<<NEWLINE>>>` markers. Inspect
  the existing oracle output before changing semantics or the importer.
- 10 empty-name diagnostics. Grammar dumps already match; runtime error
  reporting was incorrectly adding source positions to `install("")` errors.
- 5 `str2lang` type/length errors. Port `coerce.c:do_str2lang`'s messages
  and builtin call selection.
- 3 other diagnostic/escape cases, including unterminated `%` operators
  and an empty name made through `as.symbol`, which belongs to lang.

Changes port `source.c:parseError`, `getParseContext`, `tabExpand`,
`gram.y:raiseParseError`, `SpecialValue`, and `coerce.c:do_str2lang`.
Grammar diagnostics, parser conditions and ordinary R errors retain their
separate reporting paths. Error calls own a copy of the relevant syntax
only when an error occurs, so they survive arena teardown. Text buffers
append a newline per element. `parse(n=0)` reads nothing, and positive
counts stop before parsing an invalid following statement.

Review corrections to the recovered work: expand tabs independently on
each context line; omit the filename colon when no filename is known;
use `getFilename`'s environment-only rule for condition filenames; preserve
`n=NA` as all expressions on file connections; avoid copying pipe trees
on successful parses. The pre-rebase syntax unit tests and curated parser
oracle passed. Rebase retained main's arena AST and console parser.

Result: 147/191 -> 168/191 (+21), zero recorded-pass regressions.
`--update` records 22 newly listed passes, including one already passing
on the baseline. Syntax unit tests, curated parser oracle, runtime parser
tests, and both runtime integration tests pass. The complete runtime unit
suite has one pre-existing host-libc difference in
`builtins::complex::tests::c_library_oracle` (sqrt, one bit); all 28 other
unit tests pass. The PR's Debian CI build and tests pass. A host-linked
binary cannot run in the older Debian image (GLIBC_2.44 dependency), so
that attempted cross-libc test run was rejected, with no code changes.

Remaining 23 filter failures: 11 parse-data/source metadata cases;
9 CLI empty-backtick reports and one CLI octal-escape report that still
append console context; empty-string assignment and `as.symbol("")`
need the language track. Next CLI step: consume `ParseError.style` in
`echo.rs`, preserving condition calls through runtime error reporting.
The imported NEWLINE cases improve through correct `<<` token consumption
and error context; no importer or expected-output changes were needed.

Commands for verification:

```sh
export R_HOME=$HOME/.local/opt/R-4.5.0
scripts/corpus.py run fastr --filter 'parser/TestParser|TestBuiltin_(parse|deparse_parse|str2lang|str2expression)|TestSimpleParsing' --jobs 4 --keep-output --update
scripts/parse-corpus.py check
cargo test --release -p robigo-syntax -p robigo-runtime
scripts/corpus.py run gnur-tests --jobs 4 --update
cargo fmt --check
```

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
- srcrefs: not dumped (keep.source = FALSE); if the dump ever includes
  them, byte positions follow xxbyteno.
- Throughput, next steps: fastr's per-file setup dominates its 240k
  instr/kB (Parser construction, the token Vec, per-file Arena
  chunks); a scratch-reuse pass over args Vecs and the per-call Vec in
  `call2` is the next measured lever, and the remaining allocator
  traffic is the args/formals scratch vectors plus the token stream.
- Invalid UTF-8 at the console (`invalid multibyte character in
  parser`, gram.y's `mbcs_get_next`, which also makes a truncated
  multi-byte char at end of input become END_OF_INPUT): robigo's
  &str API cannot see it; callers today feed lossy conversions. A
  bytes-based lexer entry would close it (validate each multi-byte
  char in lexer order so a grammar error earlier in the chunk still
  wins), plus the R_ParseContext byte-ring slicing the cli already
  does lossily.

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

## 2026-10-28 (later): parse throughput — callgrind 202,567 -> 103,241 instr/kB, wall 25.6 -> 51.7 MB/s

- why: the track's throughput metric; every step measured with
  `scripts/parse-perf.py cg` (callgrind instr/kB over all six corpora,
  the machine's contention makes wall clock noisy) with the corpus
  ratchet re-run at 100% (41,067 sources) after each change.
- steps (kept, callgrind instr/kB):
  1. 202,567 -> 167,442: the verified part of the PR-8 WIP
     (`origin/track/parser-wip-after-pr8`): tokens borrow the source
     (Sym, %op% as &'a str), parser interns names as Rc<str> (FNV),
     the infix loop takes the token by value. The WIP's warning-text
     changes were NOT kept (main's texts are the verified ones).
  2. -> 159,607: parse-batch feeds valid UTF-8 into String::from_utf8
     (no lossy copy), CR-strip returns the input Vec when there is no
     CR, checksum folds 8 bytes at a time (harness overhead down).
  3. -> 156,929: lexer scans whitespace/comments/identifiers/numbers
     on a local index over the source bytes (bounds checks elided;
     comment ends still lower to memchr).
  4. -> 141,907: the parser peeks in place — expect/program/brace_body/
     args/formals/member_name/for-header match discriminants in the
     token stream and only move what they keep, by bump. The ratchet
     caught a missing newline skip at the top of the args loop during
     the rewrite (`f(a,\n b=1)` parsed `b=1` as an `=` call).
  5. -> 138,180: operator call names resolve once per parse (indexed
     by a small Op code), and checkForPlaceholder's walk is skipped
     when no `_` token was lexed.
  6. -> 137,985 with [ and [[ through that table; a byte-level rewrite
     of next_token's dispatch measured WORSE (138,834) and was
     reverted — the char match with peek_char's ASCII fast path is
     already cheap.
  7. -> 136,147: an integer fast path for decimal literals
     (`R_strtod5`'s exact steps in u128 arithmetic for <=19 digits
     with the exponent in [-22,22]; everything else keeps the LD
     emulation). 200k randomized + boundary cases check fast and slow
     bit-for-bit equal; GNU R's values are pinned by tests/parse/
     literals.R and the corpus.
  8. -> 103,241: the arena AST. Expr<'a> borrows names and strings
     (identifiers and escape-free strings point into the source;
     operator names are &'static), and call nodes, argument slices and
     formals live in a bump arena whose alloc takes &self (bumpalo's
     trick) so the recursive descent can build nodes anywhere. No Box,
     Vec or Rc per node; parse_program/parse_console take &Arena and
     the runtime/cli create one per parse site.
- wall clock (contended, load ~2.3 of 2 cores): 25.60 -> 51.65 MB/s
  total (rlib 61.5, cran 66.7); GNU R 4.5.0 parses rlib at ~6.5 MB/s
  by the same kind of loop (Rscript + readLines + parse, startup
  included), so robigo is ~9-10x GNU R's parse throughput on that
  corpus.
- fastr's per-source overhead (31k tiny snippets, 240,640 instr/kB vs
  rlib's 99,828) is now dominated by per-file setup, not scanning.
- note: the shared `~/.cache/cargo-target` once bled another
  worktree's stale robigo-syntax rlib into bin builds (lib compiles,
  bin cannot find `dump_error`); this track builds with a private
  `CARGO_TARGET_DIR=~/.cache/robigo-tgt-parser`.

## 2026-10-28 (later still): `parse_console` for the cli track

- why: docs/ledger/cli.md asked robigo-syntax to own the console
  grammar; echo.rs carried three workarounds (ends_in_comment, a
  first-token `;` check, the terminator requirement).
- what: `parse_console(&Arena, &str) -> ConsoleStatus` is
  `R_Parse1Buffer` with gend = 0 (`// R: main.c:Rf_ReplIteration`,
  gram.y's `prog` rules, `R_Parse1`): one chunk — the input up to and
  including the first `;` or newline, wherever it lies — is Null
  (bare newline), Ok (one *terminated* statement + the lexer's
  warnings, which GNU R's GenerateCode gate keeps to complete
  statements), Incomplete (an error at end of input; a buffer ending
  inside an unterminated comment, SkipComment's `EndOfFile = 2`),
  Error, or Eof (empty buffer). The lexer now reports the
  unterminated-comment flag (`tokenize_with_flags`).
- probes: statuses pinned by unit tests from `R --vanilla` stdin runs;
  interesting finds: `1 # c;` (the REPL's cut inside a comment) is
  *transiently* INCOMPLETE — the rest of the line (`\n`) completes it
  on the next chunk with no `+` prompt in the transcript, because the
  prompt only prints when a new line is read; `x |> _` is "The pipe
  operator requires a function call as RHS" (not the placeholder
  message); `f(x, _)` is "invalid use of pipe placeholder" at
  (<input>:1:0).
- the `1e-3L` warning choice (`seendot && !seenexp`) and `1;;2` file
  mode were re-verified against GNU R and were already right on main.
- echo.rs now delegates (ends_in_comment/first_semicolon deleted);
  the 7 tests/r/cli conformance scripts and a gnur-tests echo slice
  pass (simple-true newly passes).

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
