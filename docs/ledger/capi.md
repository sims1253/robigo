# capi ledger

The measuring stick for `robigo-capi` (G2): a census of libR's exported
API as installed packages actually use it, and a corpus of C cases whose
GNU R output is the target. Metric: weighted census coverage — the sum
of `n_pkgs` over symbols a corpus case imports, over the total.

## 2026-10-27: the census and the corpus

- why: before `robigo-capi` can be hill-climbed, both its target and its
  progress measure must exist; GNU R is the oracle and robigo loads no
  native code yet.
- census (`scripts/capi-census.py scan` → `tests/capi/symbols.tsv`):
  831 libR symbols imported by 24 GNU R + 79 CRAN packages, total weight
  5339 (sum of per-symbol package counts). `tools:::funAPI()` supplies
  the API column; headers are found by word-matching the include tree.
- corpus (`tests/corpora/capi`, wired into `scripts/corpus.py` with a
  per-corpus `build` hook that runs `R CMD SHLIB`): 20 cases, each an R
  driver (`cases/<name>.R`) over a C source (`support/<name>.c`), built
  artifacts gitignored. Areas: allocation/PROTECT, vectors and
  accessors, strings and encodings, attributes, pairlists and calls,
  coercion and predicates, environments/symbols, evaluation, errors and
  warnings, non-local exits (`R_UnwindProtect`), external pointers and
  finalizers, ALTREP, routine registration, C callables, Rmath and the
  RNG, S4/closures/promises, R's Fortran (LINPACK QR/Cholesky, vmmin/
  nmmin/optif9, Rdqags, intpr), serialization.
- result: coverage 0/831 symbols, 0% (nothing existed) → **398/831
  symbols, 4848/5339 = 90.8% weighted**; robigo passes 0/20 (expected,
  G2 not started). All recorded output deterministic: 0 unstable, 1
  masked line (`R_signal_reprotect_error` prints an uninitialized index
  in GNU R itself). Kept.
- R API quirks the cases now pin down (each cost a crash or a wrong
  output until read in R's sources): `mkNamed` stops at an *empty
  string*, not NULL; `R_existsVarInFrame(rho, sym)` but
  `R_removeVarFromFrame(name, env)` — opposite orders;
  `VectorToPairList` only accepts lists (`VECTOR_ELT` unconditionally);
  `installChar` is a macro for `Rf_installTrChar` (`Rf_installChar` is a
  separate export); `type2str` returns a CHARSXP; `R_forceSymbols(dll,
  TRUE)` *refuses* name-based calls (by-name lookup skips the DLL,
  Rdynload.c:1168) — call via `getNativeSymbolInfo(name, PACKAGE=dll)`;
  `R_FindSymbol` with a NULL package segfaults (`strcmp(NULL, ...)`,
  pass `""`); `R_isForkedChild` is data, not a function;
  `R_InitOutPStream` stores NULL callbacks as-is (write through
  `R_WriteConnection`); `.External` receives the routine name as the
  first pairlist element; `...` binds to `R_MissingArg` when empty and
  to a `DOTSXP` pairlist otherwise.

## Audit of libR.so's memory safety (2026-11-02)

- `tests/bridge.rs` drives the real bridge with an `Interp` and Rust
  routines, for what the stub host of `tests/capi.rs` cannot reach. One
  thread per process, so a case that must abort runs in a child.
- Confirmed by a failing test before the fix: conversions after a routine
  (SIGABRT), SET_TRUELENGTH, finalizers with the stack scan off, a second
  thread overwriting R_NilValue, NULL continuation and CLEAR_ATTRIB on a
  CHARSXP (both SIGSEGV), SET_BODY/SETLENGTH freeing what they replace,
  the finalizer guard stuck after a jump.
- What C replaces in a runtime object lives until top level, not until the
  outermost native call returns: the frames reading it can be above that
  call (`lapply(x, function(e) .Call(h, x))`). A long-running function
  that keeps replacing attributes through C holds the old ones until it
  returns. Element stores (`set_*_elt_through_alias`) drop the old element
  at once; a call element running while C replaces it is still exposed.
- Normal `.Call` under callgrind (three arguments, release, 100k calls,
  minus a load-only run): 16,743 -> 16,795 instructions (+0.31%). Two gates
  on the conversions, the argument-count and protect-depth compares and
  republishing the interpreter. The first version cost +98: a `Result`
  around the write-back was 37 of it, now only on the rare path.
- A fixed array in `sort::internal` panicked in `extern "C"` for
  R_orderVector's keys; only the capi corpus's `vectors` case caught it.
- Shared-target trap: cargo hashes path packages relative to the workspace,
  so checkouts share build-script OUT_DIRs. A concurrent build replaced
  `librobigo_graphics.a` and new C symbols went missing at link time; a C
  change needs a private CARGO_TARGET_DIR while others build.

## Open (next steps, by uncovered weight)

- `GE*` graphics engine (~60 weight): needs a `DevDesc` device in the
  case; next agent should port R's `tests/Embedding`-style dummy device.
- `Rf_CreateAtVector` (3), R connections from C (`Rconn_fgetc` 2,
  `getConnection` 2), `R_Serialize` is covered but `R_InitInPStream`/
  unserialize from C is not.
- The 1-weight tail (~380 symbols): mostly d/p/q/r variants, `R_GE_*`
  accessors and LAPACK; grind through with a table-driven case.
- The corpus hashes only R drivers (`scripts/corpus.py`): a `.c` change
  needs `record --all` (documented in `tests/corpora/README.md`); fold
  the `.so` sha into the case hash if that ever bites.
