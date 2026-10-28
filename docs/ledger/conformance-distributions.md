# conformance-distributions ledger

`d/p/q/r` functions and random generation: nmath ports
(`crates/robigo-numerics/src/`) and the `stats` distribution wrappers
(`crates/robigo-runtime/src/native/stats/`), `builtins/random.rs`.
Metric: `fastr` passes with `--filter
'TestDistributions|TestRandGenerationFunctions'`, no regressions anywhere.

## 2026-10-28: stats RNG result types and rank-generator limits

- Baseline on `49e0ee5`: FastR expanded distribution/RNG filter **1523/1558**,
  zero regressions. GNU R regression slice **2/3**, examples **8/21**.
- Grouped the 35 FastR failures before editing:
  - 13 integer-valued generator NA/type failures, including conversion to
    real when a draw exceeds INT_MAX.
  - 3 rank-generator timeouts from saturating Rust casts where C produces INT_MIN.
  - 3 missing coercion warnings, in RNGkind and runif's length argument.
  - 7 missing rmultinom entry points.
  - 9 stats natives outside this track: 6 fft and 3 numeric_deriv.
- Ported stats/random.c's integer output and overflow promotion while retaining
  main/random.c's REALSXP results for direct `.Internal` calls. Shared-file
  change: native/mod.rs routes random aliases to the stats dispatcher.
- Ported the C int casts in rsignrank/rwilcox and fallible Wilcoxon allocation.
  Added a GNU R 4.5.0-3 oracle grid with 81 parameter/seed combinations,
  648 exact draws, including Inf and values outside the C int range.
  The recording script and data live in robigo-numerics/tests/data.
- Rejected inherited WIP's rmultinom wrapper because it was unreachable from
  the native dispatcher; removed it and its unused nmath error channel.
  Fixed inherited WIP's two-parameter arity bug and preserved direct Internal
  semantics. Its earlier 1450/1558 result was not a sound baseline.
- GNU R regression failure is reg-tests-1d's missing formals warning.
  Of 13 failing examples, 11 stop at missing graphics routines, one at
  rmultinom, and one at r2dtable. Native stats/graphics support belongs to G2.
- Result: FastR **1523/1558 -> 1542/1558**, zero regressions. Recorded 23
  additions to passing.txt, including four existing unrecorded baseline passes.
  GNU R slices remain **2/3** and **8/21**, zero regressions; recorded their
  two previously unrecorded passes. All 19 RNG failures are fixed. Remaining
  failures are seven rmultinom cases and nine FFT/numeric_deriv cases.
- Verification: new result-type conformance script and full rank RNG grid
  match GNU R 4.5.0 stdout/stderr exactly. All three conformance checks
  pass, including interpreter/VM/JIT agreement. Numerics passes in the
  oracle container: 39 unit tests, 2 rank tests, 1 doctest. The host's glibc
  2.44 fails the unchanged powl comparison; Debian glibc 2.41 passes it.
- Performance experiment: 100k normal, uniform and Poisson draws, callgrind
  including startup: baseline 341,579,745 instructions; dynamic slice
  dispatcher 348,948,579 (+2.16%). Rejected that dispatch implementation;
  kept static specialization of the generator closure. Final count is
  **340,796,787**, 0.23% below baseline, with identical output. Reproduce
  with `set.seed(123); x <- rnorm(100000); y <- runif(100000);
  z <- rpois(100000, 4); print(sum(z))` after `library(stats)`.
- Rebased onto `ad59f6f` after the values, CI and C API security merges.
  Runtime unit tests and the final FastR repeat are pending.

## 2026-10-28: baseline and failure grouping

- why: measure first, then attack the biggest root causes.
- result: **822/1284 pass** (462 failing) at `7bb554c`.
- grouping of the 462 failures (from `.robigo/results/fastr.json`):
  - **330**: missing non-central distributions — the `stats` native
    routines `dnbeta/pnbeta/qnbeta`, `dnf/pnf/qnf`, `dnt/pnt/qnt`,
    `dnchisq/pnchisq/qnchisq` are not implemented (nmath nbeta.c, nf.c,
    nt.c, nchisq.c).
  - **49**: `ptukey`/`qtukey` missing (nmath ptukey.c, qtukey.c).
  - **64**: `d/p/q wilcox` and `d/p/q signrank` missing
    (nmath wilcox.c, signrank.c).
  - **7**: `rmultinom` missing (stats/src/random.c do_rmultinom).
  - **13**: `r*` values are `NaN` where GNU R gives `NA` (and the wrong
    vector type/length): the `.Call` path goes through
    `stats/src/random.c` (INTSXP results with `NA_INTEGER`, REAL only
    after an integer overflow), not through `main/random.c`'s
    `.Internal` path that `native::is_internal_alias` sends it to.
  - **3**: `.Random.seed` not found error reports the wrong call
    (`get(x, envir, mode, inherits)` instead of GNU R's user call).
  - **3**: `rwilcox` with huge sizes — GNU R dies in `R_Calloc`; robigo
    hangs (timeout). Needs `rwilcox`.
  - **1**: `runif("hello")` must error "invalid arguments" (stats
    random.c's `isNumeric` check), robigo's message differs.
