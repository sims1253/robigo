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

## 2026-10-31/11-01: the rlang chain, fixed across two PRs

- `R_mkClosure` / `R_PromiseExpr` / the promise SET_* write-through and
  `R_IsPackageEnv` / `R_EnvironmentIsLocked` landed on main via #161 and
  #156 (implemented in parallel by the capi track / huerd dogfooding;
  this branch's own port of the first two was dropped as duplicate in
  favor of main's superset).
- Still missing on main afterwards — and fixed on this branch:
  - the bridge's `coerceVector` with a LISTSXP target: rlang's
    `enexpr`/dots machinery coerces vectors to pairlists through
    `Rf_coerceVector(v, LISTSXP)` during `.onLoad`; routed to
    `as_vector`'s pairlist arm (R: coerce.c:coerceVector1).
  - grDevices `colors()` and `col2rgb()`: testthat's describe() and
    failure reporting style through cli (`is_r_color` -> colors(),
    combine_ansi_styles -> col2rgb); without them every describe-based
    test file (graph-editing, serialization-boundary, mermaid, ...)
    stalled or cascaded failures. Ported from grDevices/src/colors.c:
    the 657-entry ColorDataBase, rgb2col (4/5/7/9-char hex), StrMatch,
    name2col, str2col palette indices (strtod truncated to int BEFORE
    the zero check, so "0.5" is index 0 — a usize::MAX panic found by
    review), inRGBpar3 element rules, the col2rgb matrix with dimnames.
- Result on this branch: dagriculture use.R byte-identical to GNU R
  (mod the startup banner), including every rlang error class.

## Oracle evidence

- use.R transcript (README example + queries + plan + tables + print +
  mermaid + 4 typed errors): byte-identical, exit 0.
- colors/col2rgb: 22-case grid byte-identical under LC_ALL=C (this
  host's GNU R localizes messages — always compare under LC_ALL=C);
  recorded as conformance `tests/r/packages/grdevices-colors.R`
  (base-only: no utils head/tail).
- dagriculture testthat census (idle machine): 13/13 files complete;
  7 files all-pass; serialization-boundary 22/22 under the package
  harness; constructors 3 fail/10 err, boundary 1 err, mermaid 5 err
  remain (detail triage below).

## Open

- constructors/boundary/mermaid failures: need detail runs on a calm
  machine (the fleet saturated this host, load 8–37, swap full, for
  most of the session; testthat under robigo is also ~10–20x slower
  than GNU R here — a perf observation for the parent's Gemini
  workload report, not chased in this PR).
- test_dir(package=) filter once matched no files for a filter GNU R
  matches ("No test files found", test-constructors) — not reproduced
  when calm; revisit if it recurs.
