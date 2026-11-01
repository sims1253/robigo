# conformance-examples: help-page examples and pure-R CRAN packages

Metric: passing cases in `gnur-examples` (examples of every help page in GNU R 4.5.0's base and recommended packages) and `cran-purer` (pure-R CRAN packages: R6, withr, assertthat, proto, R.methodsS3).

Baseline on main (`f523d79a`):
- `gnur-examples`: 1,046 / 1,153 passing (90.7%)
- `cran-purer`: 36 / 41 passing (87.8%)

---

## 2026-10-31: pcre_config, nargs out-of-context, OutDec print formatting

- **why**: Help-page examples in base R and withr failed on missing internal, incorrect default for out-of-context primitive, and unhandled option formatting:
  - `gnur-examples: base/pcre_config`: GNU R's `pcre_config()` delegates to `.Internal(pcre_config())` in `grep.c:do_pcre_config`. It was completely missing in robigo, aborting with `there is no .Internal function 'pcre_config'`.
  - `gnur-examples: base/nargs`: In `lang.rs:do_nargs`, when called outside a function context (`frame_of_env` returns None), robigo returned `0`. GNU R's `util.c:do_nargs` initializes `nargs = NA_INTEGER;` and only updates it inside a closure context, returning `NA_integer_`.
  - `cran-purer: withr/man/with_options`: Tested decimal separator `options(OutDec = ",")`. Robigo initialized the option but ignored it during vector, matrix, and complex printing in `print.rs` and `format.rs`.
- **what**:
  - `builtins/grep.rs`: registered and implemented `.Internal(pcre_config())` returning a 4-element logical vector `c("UTF-8" = TRUE, "Unicode properties" = TRUE, "JIT" = TRUE, "stack" = FALSE)` as GNU R's `do_pcre_config` in `src/main/grep.c`.
  - `builtins/lang.rs`: `do_nargs` returns `NA_INTEGER` (`i32::MIN`) when called outside a function frame.
  - `format.rs`: added `out_dec: String` (default `"."`) to `PrintParams`, and added `encode_real_dec` and `encode_complex_dec` substituting `.` with `out_dec` when `out_dec != "."`.
  - `print.rs`: wired `Interp::print_params` to read the `OutDec` option into `pp.out_dec`, and passed it through to `encode_real_dec` / `encode_complex_dec` across vector and matrix printers.
- **tests**:
  - `gnur-examples`: `base/pcre_config` (passed, ratcheted)
  - `gnur-examples`: `base/nargs` (passed, ratcheted)
  - `cran-purer`: `withr/man/with_options` (passed, ratcheted)
  - Verified zero regressions across `cran-purer` (37/41 pass) and all 43 `robigo-runtime` unit tests pass.
- **result**:
  - `gnur-examples`: 1,046/1,153 -> 1,048/1,153 (+2 cases)
  - `cran-purer`: 36/41 -> 37/41 (+1 case)

---

## 2026-11-01: round 2 — `.Internal(file.show())` and `.Internal(file.symlink())`

- **why**: `gnur-examples` help pages failing on two missing internals from
  `platform.c`:
  - `file.show` (used directly by `base/file.show`; `utils/promptPackage` and
    `utils/write.table` call it on the files they write) aborted with
    `there is no .Internal function 'file.show'`.
  - `file.symlink` (`base/files`) aborted with
    `there is no .Internal function 'file.symlink'`.
- **what**:
  - `builtins/sysutils.rs`: ported `do_fileshow`
    (R: `platform.c:do_fileshow` + `sys-std.c:Rstd_ShowFiles`): argument
    checking in C's order (`asBool2(delete.file)` first, then the string
    checks), `isValidStringF` semantics for `pager` (NA or empty first
    element ⇒ `""`, not an error), temp file via `R_tmpnam(NULL, ...)` (empty
    prefix), per-file `

` headers, `Cannot open file '%s': %s` message,
    then `R_system("'pager' < 'tmp'")`; status 127 warns
    `warningcall(R_NilValue, "error in running command")` (robigo:
    `warningcall(&Value::Null, ..)`). Visible vs invisible per `names.c`:
    `file.show` eval col `111` (invisible), `file.symlink` col `11`
    (visible — the wrapper's return value must auto-print).
  - `do_filesymlink` (R: `platform.c:do_filesymlink`): recycling
    `from`/`to`, `nothing to link` when `from` is empty, `logical(0)` when
    `to` is empty, NA ⇒ FALSE, warning
    `cannot symlink '%s' to '%s', reason '%s'` on failure.
- **environment note** (host ≠ oracle): the corpus was recorded in the
  `robigo-oracle:4.5.0` docker (Debian trixie, `r-base-core 4.5.0-3`,
  R_HOME=/usr/lib/R, R_DOC_DIR=/usr/share/R/doc, PAGER=/usr/bin/pager→more).
  CI's corpora job runs the same container, so marked cases behave like the
  oracle there. On this host the system `/usr/share/R` is 4.6.1's, so ~20
  path-printing cases (Rhome, search, srcfile, getLoadedDLLs, ...) fail
  locally regardless of code; and `file.show`'s group needs
  `PAGER=more R_DOC_DIR=$R_HOME/doc` locally (files md5-identical to the
  oracle's; `more` is the same program the oracle resolves to).
- **tests**:
  - `gnur-examples`: `base/file.show`, `base/files`, `utils/promptPackage`,
    `utils/write.table` (passed under oracle-aligned env, ratcheted).
  - `gnur-examples`: 9 more cases now pass and were ratcheted —
    `base/maxCol`, `datasets/{AirPassengers,Nile,UKDriverDeaths,nottem}`,
    `stats/{arima,arima.sim,arima0,predict.arima}` — fallout of merged #158
    (`do_vector` alignment), unmarked by that PR; all pass under the plain
    `R_HOME`-only env.
  - `cran-purer`: 36/41 locally, 0 regressions (#160's withr case is on that
    branch, not main).
  - `cargo test -p robigo-runtime --lib`: 81 pass; 2 failures are
    `builtins::complex::tests::c_library_*_oracle` (bit-level libm drift,
    complex.rs untouched here, another agent's active topic).
- **result**: `gnur-examples` 1,046/1,153 → 1,059/1,153 marked (+13);
  CI is ground truth for the ~20 host-env-sensitive cases.

## 2026-11-01: round 2 addendum — memory caps

Conductor rule after an OOM incident: every corpus/test command now runs
under `systemd-run --user --scope -q -p MemoryMax=3G -p MemorySwapMax=0`,
corpus runs with `--jobs 2`. Applied from the first full-corpus run in this
round onward.

---

## 2026-11-01: round 3 — top-level task callbacks (main.c)

- **why**: `base/taskCallback{,Manager,Names}` abort on the missing base
  native `.C_R_addTaskCallback` ("native routine 'R_addTaskCallback' of
  package 'base' is not implemented yet").
- **what**: `runtime/src/task.rs` (new) ports main.c's task-callback
  machinery: `R_addTaskCallback` / `R_removeTaskCallback` /
  `R_getTaskCallbackNames` as base native ports; `Rf_callToplevelHandlers`
  + `R_taskCallbackRoutine` as `Interp::call_toplevel_handlers`; the hook
  in `Rf_ReplIteration`'s PARSE_OK branch (echo.rs) fires after the value
  is printed, only on successful evaluation. Self-removal (PR#18508) is
  postponed via an id check; callback errors are swallowed (R_tryEval,
  frame/handler save-restore like `run_dot_last`); warnings from a
  callback print with the REprintf prefix. Cost when unused: one empty-Vec
  check per top-level expression.
- **tests**: 3/3 taskCallback cases pass and are ratcheted; slice of
  echo-path cases + `cran-purer` 36/41, 0 regressions; runtime unit tests
  unchanged (81 pass, 2 pre-existing complex libm-oracle failures).
- **result**: `gnur-examples` 1,060 → 1,063 marked. Branch
  `track/conformance-examples-taskcallbacks` (single commit on origin/main).

## 2026-11-01: diagnosis — SS*/nls family diverges via numericDeriv (NOT yet fixed)

~12 cases (`stats/{SSasymp,SSasympOff,SSfpl,SSgompertz,SSmicmen,SSweibull,
NLSstAsymptotic,getInitial,numericDeriv,predict.nls,power,stepfun}` + 4
nlsModel cases): `getInitial` returns wrong values. Bisected to a 4-line
repro (fails in robigo, gradient is all zeros; GNU R gives 6):

```r
e2 <- new.env(); e2$g <- 3
numericDeriv(quote(g^2), "g", e2)   # value 9, gradient 0 (should be 6)
```

- Root cause: stats' C `numeric_deriv` (nls.c) does `R_NewEnv(rho)`,
  `defineVar(name, duplicate(par), rho1)` and then perturbs the parameter
  **in place through the C REALSXP pointer** between `eval(expr, rho1)`
  calls. Robigo's bridge marshals robigo values to C by copy, so the
  in-place perturbation is invisible to the next eval: every forward
  difference is 0, the QR is rank 0, plinear nls "converges" at once
  (criterion 0.00e+00, stays at start), and `getInitial` returns garbage.
- Works when `rho` is a closure frame (bindings re-read live), fails for
  fresh `new.env()`s; also reproduces deep inside `nlsModel` — exactly the
  SS* path. A second, related bug surfaced while instrumenting:
  `native/linpack.rs:250` `dqrsl` panics (`index out of bounds: len 0`)
  on a rank-0/0-column QR call via `qr.coef` (GNU R's dqrsl.f has the same
  y(1) indexing but is reached only with n>=1; robigo reaches it with
  empty vectors) — panics are always robigo bugs.
- Fix direction (next round, C-API layer, my charter): make
  `defineVar`-from-C into a C-created env bind the C-owned vector (the
  `c_owned_binding`/holder machinery in native/mod.rs exists for exactly
  this), so in-place C mutations are visible to subsequent evals; and
  guard `dqrsl`'s ju==0 branch against empty inputs.

## 2026-11-01: rounds 4-6 (numericDeriv, La_qr, Sys.umask) — shipped

- **numericDeriv / SS* family** (PR #177, merged): `defineVar` from C into
  a C-created env binds the C-owned vector through `c_owned_binding`, so
  stats' `numeric_deriv` perturbation is visible to the next eval;
  `dqrsl`'s empty-input path guarded. SS*/nls cases pass (12-16 cases).
- **La_qr / qr_coef_real / qr_qy_real** (PR #179, open): dlsym'd system
  LAPACK (`dgeqp3`/`dormqr`/`dtrtrs`), pivoted dimnames like Lapack.c;
  `base/qr` passes. CI: gnur-tests job failed once — local diagnosis
  below.
- **Sys.umask + attribute-of-primitive warning** (PR #182, merged):
  `platform.c:do_sysumask` (NA queries, visible; set returns previous
  mask invisibly, class "octmode") and `attrib.c`'s deprecation warning
  on `attr<-`/`attributes<-` of primitives. `base/octmode`, `base/attr`
  pass. Metric after #182: gnur-examples 1,067 marked on main.

## 2026-11-01: diagnosis — PR #179's gnur-tests CI failure

The gnur-tests CI job (46-minute step timeout) failed on the laqr branch.
Local reproduction with per-case scopes (`MemoryMax`, `oom.group` kills a
whole scope, so one scope per case) found three cases:

- `reg-large` and `utf8`: the corpus invocation itself was killed — a
  robigo process ate ~2 GB+ in seconds (two in parallel at --jobs 2).
- `reg-packages`: regression — the R CMD build flow produced no
  `myTst_1.0.tar.gz`; the branch base predates #172-#178, so this is
  suspected stale-base, not the lapack code.

Next: rebase #179 on main, rebuild, rerun the three cases. If the
explosions persist on the rebased branch, they are real (qr/La_qr is the
only diff) and need a fix before merge; `reg-packages` should resolve.

## 2026-11-01: round 7 — C-assembled closures (PR #185)

- **why**: `stats/deriv` printed `<pointer: 0x0>` for
  `deriv(..., function.arg=...)` and died there; `stats/selfStart`
  aborted at the same place. stats' `deriv` builds its gradient function
  in C (`deriv.c`: `allocSExp(CLOSXP)` + `SET_FORMALS`/`SET_BODY`/
  `SET_CLOENV`), and the C bridge's `from_sexp` had no CLOSXP arm — the
  node came back as a wrapped external pointer.
- **what**: `from_sexp` reads CAR=FORMALS, CDR=BODY, TAG=CLOENV into a
  runtime closure (same shape as the C-assembled promises of PR #174);
  a non-pairlist formals or missing CLOENV keeps the node opaque.
- **tests**: new capi corpus case `closure` (C closure with missing-arg
  formal and default, printed/called/inspected), oracle-recorded in
  `robigo-oracle:4.5.0`; capi 24/24. gnur-examples +2 (stats/deriv,
  stats/selfStart); full-corpus sweep: 0 real regressions (21 local
  failures are host-env: library paths, pager, NEWS/AUTHORS contents —
  all marked and green in CI); cran-purer 37/41; `cargo test -p
  robigo-capi` green.
- **result**: gnur-examples 1,065 → 1,067 marked on this branch
  (1,067 on main after #182 + this PR's 2).

## 2026-11-01: PR #179 rebased and verified — CI failure was stale-base

Rebased #179 onto c2db7c84 (post-#182 main; local head c5e0ce0d), rebuilt and
re-examined the three locally-suspect cases:

- `reg-large` / `utf8` OOM under a 3G per-case scope on any build: they are
  memory hogs by design (utf8 makes 1.1M single-char strings and sapplys a
  tryCatch over them). Main's CI artifact (PR #182 run 41314633803) shows
  reg-large 197s / utf8 29s on a 16G runner, both already failing as
  *unmarked* cases — not laqr's doing (neither script mentions qr/LAPACK).
  The 46-minute gnur-tests job death on the old head (no failed step, no
  artifact: job-level timeout) matches the old base 6c894831 (#165, before
  the perf/conformance merges #166-#182), not the lapack code.
- `reg-packages` still fails locally on the rebased branch, but it failed
  identically with the pre-laqr cclosure binary: `build.pkg("myTst")`
  produces no tarball on this laptop (R CMD build subprocess flow, host
  env). It passes in CI on main (3.0s) — CI arbitrates.
- Local sweep with the rebased binary: all 32 marked gnur-tests cases
  runnable here pass (lapack, lm-tests, reg-BLAS included), `base/qr`
  passes in gnur-examples, cran-purer 37/41 with 0 regressions.

Conclusion: pushed the rebase (force-with-lease) and let CI confirm.
