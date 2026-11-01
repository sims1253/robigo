# core-packages ledger

CRAN packages with native code: install them from source against
robigo's headers (the source-compatibility contract, `docs/core.md`) and
run their examples/tests with GNU R's output as the target.

Metrics, in order:

1. `capi` corpus (`scripts/corpus.py run capi`, 22 cases on main at
   `674f277f`, all passing after #161).
2. Weighted census coverage of libR's exports
   (`crates/robigo-capi/coverage.py`, from `tests/capi/symbols.tsv`).
3. The gate: `fastmap` then `digest` install from source inside robigo
   (`docs/core.md`, exit gate G2).
4. Popular CRAN packages with compiled code that install and whose
   examples/tests give GNU R's output.

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
