# core-packages ledger

CRAN packages with native code: install them from source against
robigo's headers (the source-compatibility contract, `docs/core.md`) and
run their examples/tests with GNU R's output as the target.

Metrics, in order:

1. `capi` corpus (`scripts/corpus.py run capi`, 23 cases on main after
   #171, all passing).
2. Weighted census coverage of libR's exports
   (`crates/robigo-capi/coverage.py`, from `tests/capi/symbols.tsv`).
3. The gate: `fastmap` then `digest` install from source inside robigo
   (`docs/core.md`, exit gate G2).
4. Popular CRAN packages with compiled code that install and whose
   examples/tests give GNU R's output — `scripts/cran-gate.py`, the
   manifest `tests/cran/packages.tsv` is the ratchet.

## 2026-11-01: baseline and the last census symbols

- Measured on `674f277f`: capi 22/22 pass; census 773/831 symbols,
  weighted 5281/5339 = 98.9%; 25 symbols left, all weight 1 (the
  `ptr_*` function pointers, graphics/HTTPD entry points, `do_*`,
  `R_runHandlers`). The gate packages have not been tried yet.
- The 58 weight-1 remainder is *not* where packages fail: a real package
  fails on behaviour of the symbols that exist (argument checking,
  attribute rules, encodings), not on a missing `R_withCallingErrorHandler`.
  So: take the cheap remaining symbols that CRAN packages do import in
  bulk (`R_qsort*`, `Rf_substitute`, the environment predicates, streams),
  then switch to the gate.
- Done (this PR): `R_tryEvalSilent`, `R_LockEnvironment`,
  `R_EnvironmentIsLocked`, `R_PackageEnvName`, `R_IsPackageEnv`,
  `Rf_substitute` (ports `coerce.c:substitute`/`substituteList` verbatim,
  including the `R_GlobalEnv` no-substitute rule and `...` splicing),
  `IS_SCALAR`, `XTRUELENGTH`, `LOGICAL_IS_SORTED` and `LOGICAL_NO_NA`
  (the last was a stub; the first two are exported by libR even though
  R's own headers implement them inline),
  `R_qsort`/`R_qsort_int`/`R_qsort_I`/`R_qsort_int_I`/`Rf_cPsort`
  (the Singleton quicksort of `qsort-body.c`, one generic body shared by
  the four entry points, so index/sort behaviour matches GNU R exactly,
  including the "ties stay put" property packages rely on), and
  `R_InitInPStream`/`R_Unserialize`. `R_Unserialize` scans the item with
  the format's own reader (serialize.c's `ReadItem` grammar: XDR, binary
  and ASCII, versions 2 and 3) so that it consumes *exactly* one item and
  hands those bytes to the runtime's `unserialize`, which is where the
  object is built; the scan is the whole point, since the caller's stream
  position must be where the next item starts.
- Result: capi 22/22 still; census 786/831 symbols, weighted
  5294/5339 = 99.2%. Kept.
- Not done: those exports have no corpus case yet (the corpus compiles
  each case against both GNU R and robigo and records GNU R's output;
  `R_InitInPStream` is the one the capi ledger already lists as
  uncovered). The unit test in `crates/robigo-capi/tests/capi.rs` pins the
  stream consumption with a hand-written XDR stream, which is the part
  that can silently break.

## 2026-11-01: the CRAN gate harness (metric 4)

- Both gate packages pass on current main: fastmap 1.2.0
  (tests/testthat, 341 expectations) and digest 0.6.39 (inst/tinytest,
  325 expectations) install with `robigo CMD INSTALL` from source and
  match GNU R 4.5.0 exactly.
- Shipped the metric-4 harness so this stays true: `scripts/cran-gate.py`
  (run/try/install/test/record/list), the manifest
  `tests/cran/packages.tsv` + GNU R's recorded `tests/cran/expected.json`
  (written only by `record`, which runs GNU R in `robigo-oracle:4.5.0`,
  never robigo), and `crates/robigo-capi/gate/shim.R` — a base-R-only
  tinytest/testthat shim that counts expectations, identical under both
  implementations. Wired as the `cran` CI job.
- Proof: fresh-cache `run` (fetch → install → test → compare) 2/2 match;
  tampering expected.json (341 → 999) makes `run` fail with exit 1.
- Next: `try` the popular set (rlang, vctrs, cli, glue, jsonlite, Rcpp,
  data.table, magrittr, R6, stringi), fix the most-unlocking blocker,
  add each package to the manifest as it passes.

## 2026-11-01: lazy-load promises force in their own environment

- The other half of WIP b2098e9b. `do_lazy_load_db_fetch` evaluated a
  PROMSXP's code in the global environment; R's
  `serialize.c:do_lazyLoadDBfetch` calls `eval(val, R_GlobalEnv)`, and
  `eval.c`'s PROMSXP case (`ENSURE_PROMISE_IS_EVALUATED`) forces a
  promise in its own PRENV — the `R_GlobalEnv` argument never reaches
  the promise. A database built from an environment whose promises were
  created with `delayedAssign(..., eval.env = e)` therefore forced its
  code in `e`; the old port looked up symbols in the global
  environment and failed.
- Both fetch paths (the flat-db cache and the per-file read) now route
  through `force_promise`, the port of eval.c's forcing (eval in PRENV,
  cache the value, clear PRENV).
- Tests: `tests/r/serialize/lazy-load-promise.R` builds a database with
  `tools:::makeLazyLoadDB` and forces a promise after `lazyLoad()`;
  full `tests/r` conformance 310/310 on the combined WIP.
