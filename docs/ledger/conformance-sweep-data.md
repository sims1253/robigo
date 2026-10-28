# conformance-sweep-data

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
