# Dogfooding: bayesgrove (2026-10-31/11-01)

bayesgrove 0.7.0.9000 (`~/sandbox/bayesgrove`): pure-R package, imports
cli, digest, dagriculture (≥0.3.0), jsonlite, rlang, S7; suggests testthat
(and brms/cmdstanr/mirai for the backend tests). GNU R is Debian's
`r-base-core 4.5.0-3` — the CI oracle version.

## Baselines

- **GNU R 4.5.0**, `R CMD INSTALL` + `testthat::test_local(load_package =
  "installed")`: **FAIL 0 | WARN 7 | SKIP 7 | PASS 2760** (skips: cmdstanr
  not installed ×5, dagitty ×1, dev-session-only ×1). Install 13.6 s.
- **robigo** (main 55befe4a + the rlang fixes below + base-package natives
  via `scripts/build-natives.sh tools utils grDevices graphics splines
  grid parallel stats`, `ROBIGO_NATIVES=.robigo/natives`):
  - `R CMD INSTALL` succeeds in 7.3 s, `library(bayesgrove)` loads, all six
    imports + testthat load, and the suite runs. First blocker hits at
    `api-boundary` (fixed below), then a segfault in the brms fit path
    (#167) stops the run; everything before it passes.
  - Without the tools natives nothing installs (`parseRd`/`Rmd5`, #168).

## Fixes

- **`capi: bridge C-assembled promises`** (branch `fix/bayesgrove-compatibility`):
  rlang 1.3.0's `env_clone` (testthat's `test_env(package)`) rebuilds a
  namespace's lazy bindings as fresh `Rf_allocSExp(PROMSXP)` + `SET_PR*` +
  `defineVar` promises; `from_sexp` had no `PROMSXP` arm, so every rebuilt
  binding read back as an external pointer and `test-api-boundary.R` failed
  with `could not find function "bg_validate_api_boundary_registry"`.
  Oracle: capi corpus case `promises` (recorded with GNU R), passes
  byte-for-byte; with the fix the test file goes 25→26 PASS.

## Coordination notes

- rlang's load chain needs `R_mkClosure`, `R_PromiseExpr` and pairlist
  `Rf_coerceVector` — implemented on `fix/dagriculture-compat-1` (unpushed
  commit 1356c285 at the time of writing; verified it loads rlang 1.3.0).
  Dogfooding of any rlang-importing package is blocked on that landing.
  I started the same three fixes, found the sibling's commit, and reverted
  mine; the promise-bridging fix above is the complementary, non-overlapping
  half.
- `R_IsPackageEnv`/`R_EnvironmentIsLocked`: PR #156 (from huerd dogfooding)
  already in flight; do not duplicate.

## Open blockers for bayesgrove under robigo

1. **#167**: SIGSEGV in `from_sexp` → `wrap_core` → `attrs_from` converting
   the result of a native routine during the brms fit (`bg_run` on a
   `brms_fit` node). Deterministic; GNU R succeeds on the same script.
2. **#168**: no tools natives by default → `R CMD INSTALL` / `load_all`
   unusable without a hand-built natives set (delivery decision needed).
3. Known fleet-wide gaps that bayesgrove's suite will hit after #167:
   mirai (#146, Suggests only), parallel/PSOCK (#143, #144) for
   `test-parallel-execution.R` / `test-jobs.R`.

## Representative-use workloads (for the parent's Gemini perf pass)

- Whole-suite testthat run (67 files, 2760 expectations GNU R) — wall time
  and instruction comparison once #167 is fixed.
- `R CMD INSTALL` time: robigo 7.3 s vs GNU R 13.6 s on this 2-core box
  (same sources, `--with-keep.source`).
- `bg_run` of a small brms fit (Stan compile + 40 iterations) once native.
