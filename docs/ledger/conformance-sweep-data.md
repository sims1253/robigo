# conformance-sweep-data

## 2026-10-29: second sweep (branch track/conformance-sweep-data-2)

- Started from origin/main e3909b9 (PR #29 and the complex-matprod branch
  were merged; `as.complex(NA) %*% NaN` verified to print `NaN+NaNi`).
- Baseline on main: expanded data filter **8,386/8,425** (39 failing),
  gnur-tests 21/59, gnur-examples 602/1,149 with 11 ratchet entries failing
  for host reasons only (local `R.home("doc")` lacks NEWS.2/AUTHORS content,
  `searchpaths()` shows the local R_HOME). Recorded 75 new gnur-examples
  passes and one unrecorded fastr pass (`TestSimpleDataFrames/c4345940`).
- Kept fix: **cbind/rbind split language objects into their components**
  (coerce.c:coerceVector → PairToVectorList; tags become names, so
  `cbind(substitute(x(y=3,z)))` gets row names y/blank). `coerceVector` on a
  symbol now warns `'symbol' object cannot be coerced to type 'list'` and
  yields NULL as coerce.c:coerceSymbol does. fastr +2 recorded
  (`TestBuiltin_cbind/9927c42`, `88301329`), filter → **8,387/8,425**,
  0 regressions; matrix conformance scripts pass except the pre-existing
  host-dependent complex_nan.R (below).
- Shared-target collision: another worktree's robigo-capi build script
  (compiling `appl/pretty.c`) reran against this tree's sources and failed;
  built with a private `CARGO_TARGET_DIR=/tmp/robigo-data-verify-target`
  from then on.

### Found for other owners (verified repro on current main)

- **perf-vm (vm.rs `element`)**: a compact-range `x` indexed with 1 returns
  the whole vector: `f <- function() { x <- 1:5; x[1] }` prints `1 2 3 4 5`
  (also `x[[1]]`, any scalar index whose value is 1, promise or literal).
  `element()` returns `x.clone()` for any non-`V` RVal at `i == 0` — right
  for scalar `I`/`D`/`L` RVals, wrong for `RVal::Range`. Global `x`,
  `x[2]`, `x[1, drop=FALSE]` and the interpreted path are unaffected.
  Fails fastr `TestSimpleVectors/9313fe3a` and likely more outside the
  corpus (base R code uses `x[i]` on ranges in closures everywhere).
- **conformance-lang (lang.rs `fast_replace`)**: complex-assignment error
  calls show the rhs *value*: `` `[[<-`(`*tmp*`, i, value = 10) `` where
  GNU R shows the *expression* `value = v` (fastr TestSimpleVectors/
  0b6df291325c0b6c, daaeabd510342a4c). `rcall` should carry `rhs_expr`.
- **conformance-values (coerce)**: `as.character(list(as.raw(1)))` must be
  `"as.raw(0x01)"` (coerceVectorList deparse1line's non-string elements);
  ours gives `"01"`, so `pmatch`'s base wrapper matches `'01'` and
  match.arg returns instead of erroring (fastr TestBuiltin_matcharg/
  85380d0ca4207480). Same file: `as.character` errors must report the
  `as.character(table)` call, not the caller's (matcharg/139373f181119b60).
- **options.rs**: `options(matprod = "default.simd")` must be accepted
  silently (gnur-tests reg-BLAS; our warning only fits builds without
  OpenMP SIMD), and `options(max.print = Inf)` must warn
  `NAs introduced by coercion to integer range` (gnur-tests print-tests).
- **misc.rs**: `asCharacterFactor` must error `malformed factor` when the
  levels attribute is missing or not a character vector (8 fastr factor
  cases, envir::as_character_factor ignores the levels type).
- **complex.rs (host-dependent)**: on this machine `cargo test -p
  robigo-runtime` fails `builtins::complex::tests::c_library_oracle`
  (sqrt ULP) and ratcheted `tests/r/matrix/complex_nan.R` differs at line 7
  (`NA NaN` vs `NA NA`) — both fail with this branch's changes stashed, and
  main's CI is green, so they look compiler/machine-dependent, not semantic.

### Still open in this slice (unchanged causes)

- fastr 37 remaining: factor 8 (misc.rs), tracemem 7 (debug.rs), `*tmp*`
  evaluation order 2 and match traceback 1 (evaluator), `[[<-` rhs display
  2 (lang.rs), unused-argument value printing 2 (arg matching), rbind/cbind
  GNU-R-segfault expectations 2 (expected status -11; not reproducible),
  parse-error truncation 2, srcref 1 (subset), lapply 4 (apply.rs),
  `x[1]`-on-range 1 (vm.rs), as.character-list 2 (coerce).
- utf8 gnur-test: `utf8ToInt` must validate bytes and return NA for invalid
  sequences; robigo stores byte strings as Latin-1 chars (lang.rs
  const_to_value), so byte-level validation needs per-CHARSXP encodings
  (value.rs) — same blocker as iconv/`Encoding<-`.

## 2026-10-28: resume after PR #29 merged


- PR #29 was already merged with passing checks. The worktree was clean,
  with no commits absent from origin/main. Rebased onto 49e0ee5 and started
  a follow-up branch.
- GNU R sources: array.c:CMATPROD_BODY, CCROSSPROD_BODY, TCCROSSPROD_BODY;
  Rcomplex.h:toC99. Leave matprod_real unchanged.
- Reproduced TestSimpleArithmetic/6f36cc66d662530c: NA printed instead of
  NaN+NaNi. The C99 recovery itself is needed for infinities; NaN operand
  selection before recovery loses the distinction between NA and NaN.
- Experiment: explicit NaN payload selection on the exceptional path.
  The first ordering failed five of 64 oracle pairs. Correcting the order
  and expanding to 625 pairs exposed eight additional real-component
  differences; corrected those too. The standalone helper now agrees on
  all 625 NA/NaN/finite classifications with Debian GNU R 4.5.0.
  Full runtime verification then exposed tcrossprod selecting the reverse
  operand order; reversing its helper inputs matches all 625 pairs too.
- Added a conformance test recorded by scripts/regen-expected.sh in
  robigo-oracle:4.5.0, including all three products and accumulation.
  An initial dput-based test exposed existing complex deparse differences;
  replaced it with component checks to isolate arithmetic.
- Residual groups inspected: tracemem is a debug builtin; factor/rep
  recursion and method argument matching belong to language dispatch;
  replacement evaluation order and printed calls belong to the evaluator;
  names(quote(x(y=z))[2]) loses the call head tag in Lang::from_items and
  requires a value.rs representation change. Those files are outside scope.
- Build environment: shared cargo artifacts initially selected stale syntax
  metadata without Arena. Touched syntax lib.rs to force rebuilding without
  changing its contents. Used the shared build lock and separate baseline
  and candidate binary copies for measurements.

Filter (the earlier branch's broader filter plus the queued regression and
crossprod):

```text
TestBuiltin_(crc64|rbind|cbind|extract_dataframe|Date2POSIXlt|POSIXlt2Date|formatPOSIXlt|strptime|asPOSIX|allequalPOSIXt|Sys|dump|filecopy|fileaccess|filecreate|fileexists|fileinfo|filepath|fileremove|filerename|factor|rep|repint|rep_len|seq|seqint|seq_len|seq_along|order|sort|sortfastpass|psort|qsort|radixsort|unique|duplicated|match|which|array|matrix|dim|subset|subset2|subset_dataframe|list|unlist|lapply|vapply|split|paste|format|substr|sprintf|nchar|strsplit|print|toString|cat|deparse|dput|makeunique|strrep)|TestSimpleVectors|TestSimpleLists|TestSimpleDataFrames|TestSimpleMatrix|TestSimpleArrays|6f36cc66d662530c|TestBuiltin_crossprod
```

- Host baseline: gnur-tests 19/59 (two unrecorded passes). r-strict-tst's
  sole difference was elapsed CPU time, 0.001 instead of 0. The base/utils
  example slice was 346/500 with 16 pre-existing recorded-pass failures,
  including platform paths, missing GNU R documentation files, and today's
  date in round.POSIXt. Do not change the oracle or remove passing entries.

- Current-main FastR baseline: **8,377/8,426**, including the queued
  arithmetic regression. The previous 8,218-case filter is **8,170/8,218**.
  Failure counts by group: library/base/TestSimpleVectors: 9, builtins/TestBuiltin_factor: 8, library/base/TestSimpleLists: 7, builtins/TestBuiltin_dim: 4, builtins/TestBuiltin_lapply: 4, builtins/TestBuiltin_rep: 4, builtins/TestBuiltin_cbind: 3, builtins/TestBuiltin_matcharg: 2, builtins/TestBuiltin_rbind: 2, builtins/TestBuiltin_split: 2, builtins/TestBuiltin_repint: 1, builtins/TestBuiltin_subset: 1, library/base/TestSimpleArithmetic: 1, library/base/TestSimpleDataFrames: 1.

- Candidate runtime now matches the complete recorded test byte for byte:
  625 operand pairs for each of three products, plus 625 accumulation
  pairs and the queued scalar regression. Runtime unit tests: 23 passed;
  lazy-load integration tests: 2 passed. Formatting passes.

- Performance experiment: branching on op inside every complex product
  raised a finite complex-matrix workload from 178,799,017 to 182,989,313
  instructions (+2.34%). Rejected that placement; move operand selection
  into the NaN-only path and remeasure before keeping the implementation.

- Moving only the branch still cost 182,921,899 instructions (+2.31%).
  Separate the exceptional handling into a cold function, keeping the four
  multiplications and two component additions inline for finite products.

- Cold extraction still cost 184,156,240 instructions (+3.00%); rejected
  as the finite-path solution. Follow array.c's finite-input split instead:
  scan for nonfinite components once and specialize the inner product loop.
  Keep the existing serial component sums; the real matrix path is untouched.
- Full expanded filter: **8,377 -> 8,378 of 8,426**. The queued regression
  is restored; all 207 crossprod cases still pass, no remaining regressions
  within the filter. gnur-tests: 19 -> 20/59 (only timing changed), examples
  unchanged at 346/500 with the same host-only failures.

- Added finite overflow/underflow oracle cases. These exposed an existing
  ccrossprod BLAS difference: ZGEMM's transposed-A branch writes ALPHA*TEMP,
  even for ALPHA=(1,0). Ported that final component multiplication from
  src/extra/blas/cmplxblas.f:ZGEMM. It matters when TEMP overflows.
- A real-matrix comparison exceeded 0.5%, but callgrind attributed the
  entire difference to different parser artifacts reused from other
  worktrees. matprod_real's component instruction counts were identical.
  Copied the cached build dependencies into a private target directory,
  forced this checkout's syntax crate to rebuild, and rebuilt both baseline
  array.rs and the candidate against those same frozen dependencies.

Reproduce the complex performance workload with R_DEFAULT_PACKAGES=NULL:

```r
x <- matrix(rep(1+2i, 2500), 50L)
for (i in 1:5) { y <- x %*% x; z <- tcrossprod(x) }
print(y[1,1]); print(z[1,1])
```

Run each binary under valgrind --tool=callgrind. The isolated build uses
CARGO_TARGET_DIR=/tmp/robigo-data-verify-target with scripts/build.sh;
its baseline restores array.rs from 49e0ee5, all other sources identical.

- Isolated final performance: complex workload **178,821,262 -> 162,054,932**
  instructions (-9.38%); real robigo/matrix
  **606,416,867 -> 606,231,030** (-0.03%). compare reports no slowdown
  above 0.5%. Final overflow cases and exhaustive component tests match
  GNU R byte for byte. The earlier PR head's full CI was green, including
  all corpora and performance.
