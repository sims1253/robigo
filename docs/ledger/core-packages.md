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
## 2026-11-01: topenv honors matchThisEnv

- Took over WIP b2098e9b from the previous agent and finished it.
  `.Internal(topenv)` dropped its second argument: `envir.c:do_topenv`
  passes `CADR(args)` (matchThisEnv, the `topLevelEnvironment` option
  that `sys.source` sets to its `envir`) into `envir.c:topenv`, where
  the walk also stops at it. Code sourced with `sys.source` that calls
  `topenv(parent.frame())` — `methods`'s `setClass(where = ...)` shape —
  must define in the sourcing environment, not the namespace behind it.
- Port: `do_topenv` reads the second argument (anything but an
  environment means NULL, as R's `target != R_NilValue &&
  TYPEOF(target) != ENVSXP` check), and `topenv_from` adds the target to
  the stop set. R's loop tests `env != R_EmptyEnv` *before* `env ==
  target`, so the empty environment is never returned even when it is
  the target; mirrored by testing for the empty env first.
- Tests: `tests/r/lang/topenv-match-this-env.R` (recorded with GNU R
  4.5.0, pins the empty-env edge); full `tests/r` conformance 310/310
  on the combined WIP; `cargo test -p robigo-runtime` unchanged (two
  `builtins::complex` libm-oracle tests fail on this machine with and
  without the change).
## 2026-11-01: metric (d) baseline; parse(text=) coercion

- Baseline (cran-gate after #175, GNU R 4.5.0 oracle vs robigo, shim
  counts): fastmap 341/0 and digest 325/0 both match (the gate is
  green). First metric-(d) probes: jsonlite 1527/11 vs GNU R 1555/6;
  R6 562/17 vs 567/12; glue 124/114 vs 106/116 (needs dep parity in
  the oracle lib before the numbers mean anything); magrittr crashes
  the shim; cli fails to install (`robigo: bzip2 compressed file
  connections are not implemented yet` building its sysdata DB);
  vctrs installs, suite slow, still unmeasured; rlang known-hard
  (#174's author).
- jsonlite's two S4 failures: `parse(text = <expression>)` — R's
  `source.c:do_parse` runs `coerceVector(CAR(args), STRSXP)` first
  (an expression or call deparses into text) and errors only when the
  argument had length but the coercion produced none. The port
  demanded an already-character `text`. Fixed in `do_parse`; pinned
  by `tests/r/lang/parse-text-coerce.R` (GNU R's own answers,
  including `parse(text = quote(a + b))` → `expression(+a, b)`).
- R6's five extra failures are all `reg.finalizer` timing: after
  `local(obj <- BC$new())`, robigo's `gc(full = TRUE)` still sees the
  object reachable (the finalizer fires at a later gc, after
  `capture.output` closed); GNU R collects it there. The minimal
  `reg.finalizer` cases all match, so this is the frame-retention
  problem the core track's ledger already tracks — left to them.
- jsonlite's remaining gap is `url()` connections (7 tests). The
  R6/S4 work above also showed `methods::setClass` under `sys.source`
  needs #196's `topenv` matchThisEnv fix: without it `where` walks
  past the sourcing environment into a locked namespace
  ("cannot add bindings to a locked environment").
## 2026-11-01: return raises the catchable no-target error at the jump site

- magrittr 2.0.5 crashed the CRAN-gate shim: `1 %>% { return(.) }`
  evaluates the block from magrittr's C code via `Rf_eval`, and the
  unconsumed `Flow::Return` reached top level uncatchably ("Error: no
  function to return from ... Execution halted"). GNU R raises that as
  an ordinary *catchable* error — `context.c:findcontext` calls
  `error()` at the jump site, inside whatever handlers are active.
- First attempt (rejected): converting the flow in the bridge's
  `h_eval`. Sol review showed both failure modes: `tryCatch(1 %>%
  {stop("boom")}, error=conditionMessage)` lost "boom" (the conversion
  intercepted valid handler-result returns crossing C), and
  `1 %>% {tryCatch(return(.), error=function(e) 42)}` gave the outer
  error instead of 42 (the conversion happened after the inner
  handler's frame was gone).
- Kept fix: port findcontext where R has it — `do_return` checks
  `frames` (rev) for a non-builtin context whose `cloenv` is the
  return's frame (closure frames push one; `do_eval` pushes one with
  `cloenv = envir`, matching R's CTXT_RETURN context), and raises
  `Flow::error("no function to return from, jumping to top level")`
  when none matches. The VM's non-body `Op::Return` does the same
  check (`compile.rs` compiles `return()` calls in closure bodies to
  the consumed `is_body` path, so only eval'd units reach it).
  Break/next keep bubbling: R's loop contexts (CTXT_LOOP) are not
  modelled yet, so `{ break }` from C still reaches top level.
- Measured: Sol's two cases match GNU R (42, "boom"); magrittr's shim
  12 passed / 13 failed (GNU R 53/0); full tests/r conformance
  310/310; capi corpus 24/24, 0 regressions; robigo-runtime unit
  tests unchanged (the two libm-oracle complex failures are
  machine-specific, identical on the base commit).
- Still open for magrittr (next blockers, in order): promises leak
  from rlang's `r_env_bind_lazy` lazy bindings ("object of type
  'promise' is not subsettable", "invalid 'envir' argument of type
  'promise'", "Can't splice an object of type <promise>"), and
  `1 %>% identity() %>% return()` returns an external pointer instead
  of 1 (the trailing bare-`return` rhs in pipe.c's masking path).
