# Ledger: dogfooding bayesim

bayesim 2.0.0 (github.com/sims1253/bayesim @ 352f082, installed for GNU R
4.5.0) — simulation framework over mirai/purrr/rlang/S7/tibble. Oracle:
host GNU R 4.5.0 (`scripts/corpus.py record` refuses anything else).

## 2026-11-02: GNU R baseline and the robigo chain

- GNU R baseline reproduced: testthat FAIL 0 / WARN 9 / SKIP 10 / PASS
  1502; the README "Try a study" representative script (20 replicates)
  reproduces byte-for-byte except `digest(result$summary)`, which differs
  between two GNU R runs (it hashes environment addresses — unstable,
  masked in comparisons).
- Chain under robigo, one fix at a time: `@<-` (merged earlier) →
  rlang's `.onLoad` needed `R_mkClosure`/`R_PromiseExpr`/
  `R_IsPackageEnv`/`R_EnvironmentIsLocked` (merged, #156/#161) →
  mirai's transport died in `request()` on `R_NewEnv(R_NilValue, ...)`
  (#146, fixed in #231 with FRAME/ENCLOS materialization) → grDevices
  natives for pillar/tibble (`scripts/build-natives.sh grDevices`,
  self-provisioning libcurl) → parallel natives for `nextStream`/
  `mc_cleanup` (`scripts/build-natives.sh parallel`) → **`library(bayesim)`
  loads and `run_simulation` succeeds with persistent daemons**.
- Remaining gaps, each filed with repros: the ephemeral mirai daemon
  path fails tasks before the data generator (random.c `invalid
  arguments`; #254), rlang's own error deparser crashes on robigo and
  hides every rlang-formatted error (#232), and miraiError message text
  differs (#233). The `promise already under evaluation` re-force
  semantics (GNU R re-evaluates an interrupted promise and warns) is
  documented in #254 as the reason bayesim's fallback error handler
  hides the real task error.
- Representative workload for the performance phase once compatibility
  closes: `tests` harness scripts in #254's repro (`run_simulation` with
  `daemons(1)`, 3×20 tasks, `seed = 42L`), plus `config_fingerprint` and
  `summarize_simulation` — all deterministic.
