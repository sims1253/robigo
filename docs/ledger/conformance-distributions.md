# conformance-distributions ledger

`d/p/q/r` functions and random generation: nmath ports
(`crates/robigo-numerics/src/`) and the `stats` distribution wrappers
(`crates/robigo-runtime/src/native/stats/`), `builtins/random.rs`.
Metric: `fastr` passes with `--filter
'TestDistributions|TestRandGenerationFunctions'`, no regressions anywhere.

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
