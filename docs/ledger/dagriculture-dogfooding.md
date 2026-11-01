# dagriculture dogfooding ledger

Metric: dagriculture (github.com/sims1253/dagriculture @ d00f3e1, v0.3.0,
installed in ~/R/x86_64-pc-linux-gnu-library/4.5) under robigo vs GNU R
4.5.0 — a representative-use transcript (README example + queries +
planning + tables + print + mermaid + the typed error taxonomy) and the
package's testthat suite (370 tests).

## 2026-10-31: baseline

- GNU R 4.5.0: use.R transcript 59 lines, exit 0; testthat 362 pass,
  8 CRAN-skips, 0 fail. Robigo (main @ 55befe4a): `library(dagriculture)`
  loads; every value, table and mermaid line of use.R already matched;
  the error taxonomy failed — `rlang::abort` needs `loadNamespace(rlang)`,
  which died in `.onLoad` on the `R_mkClosure` stub.

## 2026-10-31: fix 1 — rlang init and dots coercion (capi)

- `R_mkClosure` (dstruct.c: R_mkClosure + eval.c: CheckFormals + the
  mkCLOSXP body rules) and `R_PromiseExpr` (eval.c: bytecodeExpr(PRCODE);
  robigo promises keep the expression) in `robigo-capi/src/objects.rs`;
  the bridge's `coerceVector` routes a LISTSXP target to `as_vector`'s
  pairlist arm (coerce.c: coerceVector1; rlang's dots machinery).
- Result: use.R byte-identical to GNU R (mod the startup banner). The
  capi corpus case `objects` now matches through closure creation/call/
  read-back and R_PromiseExpr; only the SET_PRVALUE write-through
  remained. rlang imports 124 libR symbols; after this, 3 stubs remain
  (R_IsPackageEnv, R_EnvironmentIsLocked — off dagriculture's path).

## 2026-10-31: fix 2 — grDevices colors()/col2rgb() (runtime)

- testthat's describe()/failure reporting styles through cli
  (is_r_color -> colors(); combine_ansi_styles -> col2rgb); both were
  unimplemented, so every describe-based test file (graph-editing,
  serialization-boundary, mermaid, ...) stalled or cascaded failures.
- Port from grDevices/src/colors.c: the 657-entry ColorDataBase,
  rgb2col (4/5/7/9-char hex), StrMatch, name2col, str2col palette
  indices, inRGBpar3 element rules, the col2rgb matrix with dimnames.
- Oracle: 22-case grid (names incl. case/space variants, all hex forms,
  palette indices incl. fractional strings, NA/NA_, integer/real inputs,
  every error message) byte-identical under LC_ALL=C — this host's GNU R
  prints localized messages, compare under LC_ALL=C.
- Result (idle machine): 12 of 13 testthat files complete; the 13th
  (graph-editing) completed in 7s once col2rgb landed — the earlier
  "hang" was cli's error path. Machine then saturated (load >8, swap
  full) before the final census.

## 2026-10-31: fix 3 — SET_PRVALUE/SET_PRCODE/SET_PRENV write-through

- `Promise.expr` became interior-mutable (`RefCell<Value>`, ~18 read
  sites); SETCAR/SETCDR/SET_TAG on HOST proxies route to a bridge
  `setfield` op that writes the runtime promise (value/expr/env;
  SET_PRVALUE(R_UnboundValue) unforces). Completes capi corpus case
  `objects`. Build/verification pending machine calm.

## Open

- Final census under load-free conditions: test-constructors (3 fail,
  10 err), test-graph-boundary (1 err), test-graph-mermaid (5 err)
  detail runs; all other files pass (7 files all-pass; serialization
  22/22 under the package= harness).
- test_dir(package=) filter bug seen once ("No test files found" for a
  filter that GNU R matches) — reproduce when calm.
- rlang stubs R_IsPackageEnv / R_EnvironmentIsLocked (env_parent<- path;
  owned by the huerd dogfooding PR per coordination).
- Robigo runs dagriculture's testthat ~10–20x slower than GNU R on
  these files even idle — a perf observation for the parent's Gemini
  workloads, not chased here.
