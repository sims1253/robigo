# Dogfooding: huerd (dogfooding-huerd)

Track of the package-dogfooding wave (parent thread "Dogfood Robigo With
R Packages"): run the user's own packages on robigo, fix small
compatibility gaps with oracle proof, file blockers that need design
decisions. This ledger: **huerd** (github.com/sims1253/huerd), a color
palette optimizer.

## 2026-10-31: baseline and the two blockers

- package: huerd 0.6.2.9000 @ 0d21d3a (master), installed with
  GNU R 4.5.0 (`R CMD INSTALL`) into `~/R/x86_64-pc-linux-gnu-library/4.5`
  (deps already present; colorspace 2.1.4 + nloptr 2.2.1 installed from
  CRAN by the first attempt). Imports: cli, colorspace, farver, grid,
  gridExtra, nloptr, withr; Suggests: ggplot2, testthat.
- why: nobody had ever run huerd under robigo; the first two agents'
  evidence (a GNU R `test_file` log without namespace) was misread as a
  robigo failure.
- GNU R baseline: `cd tests && TESTTHAT_PARALLEL=false NOT_CRAN=true
  Rscript -e 'library(testthat); library(huerd); test_check("huerd")'`
  → **[ FAIL 0 | WARN 0 | SKIP 8 | PASS 1611 ]** in 345 s (needs
  `TESTTHAT_PARALLEL=false`: the package's parallel testthat uses
  mirai/callr, which fails on this machine — unrelated to robigo).
- robigo @ 55befe4a (triage binary, no own build): `library(huerd)`
  loads; `library(cli|withr|colorspace|farver|nloptr)` load — farver and
  nloptr run their real compiled code through robigo-capi's bridge
  (`nloptr::nloptr` optimizes; `farver::decode_colour` converts).
  Two blockers found:
  1. `library(grid)`/`library(gridExtra)` fail: grid's `.onLoad` calls
     `.Call(C_initGrid, ...)`, and grid/grDevices natives are vendored
     as routine tables without Rust ports → "not implemented yet".
     **Fix without code:** `scripts/build-natives.sh grDevices grid`
     builds them from the R 4.5.0 sources against robigo; needs a
     local libcurl dev prefix for R's configure (`apt-get download
     libcurl4-openssl-dev libcurl4t64`, `dpkg-deb -x` into
     `~/.cache/robigo-research/curl-prefix/prefix`, configure with
     CPPFLAGS/LDFLAGS pointing there — the machine lacks
     libcurl4-openssl-dev). Result: with `ROBIGO_NATIVES` pointing at
     the natives dir, `library(grid)` loads and `col2rgb` runs.
  2. `library(testthat)` fails in rlang's `.onLoad`:
     `R_mkClosure` (and `R_PromiseExpr`, `R_IsPackageEnv`,
     `R_EnvironmentIsLocked`) are capi stubs. Split with the dagriculture
     sibling (same wall, found first by them): their PR carries
     `R_mkClosure` + `R_PromiseExpr` (plus a coerceVector→LISTSXP bridge
     fix and SET_PRVALUE write-through, both theirs); this branch ports
     `R_IsPackageEnv` + `R_EnvironmentIsLocked` (envir.c) with oracle
     recordings in capi corpus case `envir`. rlang.so imports no stub
     beyond those four (checked `nm -D --undefined-only` against
     stubs.rs); vctrs.so imports only `R_mkClosure`/`R_PromiseExpr`.
- result (kept): with natives + the capi ports pending build:
  `generate_palette(3, seed=42)` under robigo is byte-identical to GNU R
  (LC_ALL=C.UTF-8) except wall-clock timestamps in the metadata —
  colors #7500E5 #EE0000 #FFFF00, metrics, 210 optimizer iterations,
  NLOPT_XTOL_REACHED from the real nloptr C code.

## 2026-11-01: after #161 (R_mkClosure/R_PromiseExpr merged) and #156 (this
track's R_IsPackageEnv/R_EnvironmentIsLocked, merged 68493386)

- `library(testthat)` under a #161 build advances past R_mkClosure and
  hits `enexpr(expr)` → "robigo: coerceVector to type pairlist is not
  implemented yet" — the fix lives in the dagriculture track's pushed
  but unmerged branch `fix/dagriculture-compat-1` (1356c285 "capi:
  R_mkClosure, R_PromiseExpr, and pairlist coerceVector"; 252b8ca0 adds
  grDevices colors()/col2rgb ports). Coordination: theirs, not duplicated
  here.
- With a build of 1356c285 (t3-e310d54a's binary) + this worktree's
  grDevices/grid natives, `library(testthat)` loads and the full huerd
  suite runs under robigo (see robigo-suite-dagri2.log; GNU R reference:
  1611 pass / 0 fail / 8 skip in 345 s).
- Full-suite pass/fail counts under robigo: pending the run's completion
  (needs the dagriculture PR merged for a mainline binary; first attempt
  was killed by a 1 h timeout with buffered output lost).

## 2026-10-31: representative-use sweep (robigo build of this branch)

- `api-sweep.R` (public API: generate_palette(4), quick_palette(5),
  evaluate_palette, is_cvd_safe, simulate_palette_cvd,
  interpret_palette_quality, export_palette css) — GNU R 4.5.0 vs
  robigo: byte-identical except R tempdir paths in one message.
- `plot_palette_analysis(p)` works under GNU R; under robigo it hits the
  rlang `R_mkClosure` stub (dagriculture PR fixes; retest after merge).
- One real conformance diff found and minimized:
  `cat(identical(1$foo, 2), "\n")` prints `Calls: cat -> identical` in
  GNU R, `Calls: cat` in robigo — issue #157 (conformance-lang's
  conditions.rs, not fixed here).

## 2026-10-31: reproducible commands

- GNU R suite: see above; log `/tmp/opencode/huerd-dogfood/gnur-full.log`.
- robigo run: `ROBIGO_NATIVES=$WT/.robigo/natives LC_ALL=C.UTF-8
  $WT/.robigo/bin/robigo gen3c.R` vs `LC_ALL=C.UTF-8 Rscript gen3c.R`
  (diff: timestamps only).

## 2026-11-01: after #161 and #156

- #161 (dagriculture) merged R_mkClosure/R_PromiseExpr; #156 (this
  track, merge 68493386) merged R_IsPackageEnv/R_EnvironmentIsLocked.
- `library(testthat)` on a #161 build advances past R_mkClosure and hits
  `enexpr(expr)` → "robigo: coerceVector to type pairlist is not
  implemented yet" — the fix sits in dagriculture's pushed, unmerged
  branch `fix/dagriculture-compat-1` (1356c285 adds the pairlist
  coerceVector; 252b8ca0 adds grDevices colors()/col2rgb ports).
  Coordinated: theirs, not duplicated here.
- With a build of 1356c285 + this worktree's grDevices/grid natives,
  `library(testthat)` loads and huerd's full suite runs under robigo
  (robigo-suite-dagri2.log; GNU R reference: 1611 pass / 0 fail /
  8 skip in 345 s). Counts pending run completion.

## 2026-11-01: performance measurements and suite status (final)

- `quick_palette(5)`: **GNU R 1.46 s vs robigo 35.7 s (~24x)** (build of
  dagriculture 1356c285, warm cache, load ~7). The suite is therefore
  hours under robigo on this 2-core machine, and long runs were killed
  by machine saturation (six parallel dogfooding agents, load 7-17);
  per-file runs verified the suite starts and executes
  (`library(testthat)` 2.0 s, `library(huerd)` 0.6 s warm / 54 s cold)
  but no full per-file counts were collected.
- Correctness evidence stands on the byte-identical
  generate_palette/api-sweep comparison above.
- Gemini workload candidates (not started): quick_palette(n) and
  generate_palette(n, optimizer = each of nloptr_cobyla/sann/
  nelder_mead/lbfgs) — nloptr objective callbacks cross the C bridge
  per iteration; evaluate_palette/cvd sweeps (farver-dominated); the
  package's own suite as a mixed workload. GNU R reference: whole suite
  345 s.
