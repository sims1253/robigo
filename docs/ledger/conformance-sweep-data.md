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
