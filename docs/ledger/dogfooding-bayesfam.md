# Dogfooding: bayesfam (dogfooding-bayesfam)

Track of the package-dogfooding wave: run the user's own packages on
robigo, fix small compatibility gaps with oracle proof, file blockers
that need design decisions. This ledger: **bayesfam
(github.com/sims1253/bayesfam)**, brms custom families — pure R over
`stats`, with `brms` and `lamW` in Imports. Resumed from the interrupted
2026-11-01 thread (its patch is PR #184; its untracked evidence script
`tests/bayesfam-use.R` lives in the sandbox clone).

## 2026-11-01: setup

- package: bayesfam 0.3.2 @ 3b5b895 (master), installed with GNU R 4.5.0
  (`R CMD INSTALL`, pure R, NeedsCompilation: no) into the user library;
  all Imports/Suggests already present (brms, rstan, lamW, gnorm, ...).
- robigo: main @ 9a54ca5e, built with `scripts/build.sh`; natives from
  the R 4.5.0 sources via `scripts/build-natives.sh` (stats, utils,
  tools, grid, grDevices, graphics, parallel, splines — the set built in
  the interrupted thread's worktree, reused read-only through
  `ROBIGO_NATIVES`). Without natives, `library(grid)`/`tools` workflows
  fail per #155/#168; PR #184 makes that error name the fix.

## 2026-11-01: representative use — byte-identical

`tests/bayesfam-use.R` (every exported `d*` density, three seeded `r*`
draws, the lookup helpers, erf/inv helpers; 15 digits, `LC_ALL=C.UTF-8`):
GNU R 4.5.0 and robigo print the same 33 lines, same empty stderr, exit
0. A load/exports/search/lazy-data check is byte-identical too,
including its warning text. Under `LANG=de_DE.UTF-8` the texts diverge —
robigo stays in the C locale: issue #170 (needs a message-translation
design).

## 2026-11-01: the testthat suite

- GNU R 4.5.0 baseline (`TESTTHAT_CPUS=1 NOT_CRAN=true test_local`): 24
  of 29 files completed all-green (dots, skips, warnings only; the
  rstan-backed family-contract/generalized_* files compile Stan and
  pass); the run dies in `test-unitlindley.R` on rstan's compile-echo
  `Error in sink(type = "output") : invalid connection` — a pre-existing
  flake under GNU R itself (re-running just unitlindley+weibull
  afterwards completes them green), before robigo is involved.
- robigo: `library(bayesfam)` (and through it brms, rstan headers, lamW)
  loads; `test_file()` per file works, byte-identical to GNU R for
  test-beta-mean.R after PR #186. Full-suite census continues after the
  #167-class brms-fit blockers (below).

## 2026-11-01: fix — srcrefs of evaluated statements (PR #186)

Symptom: a failing test reported `── 1. Error: test-custom-beta ──` under
robigo where GNU R prints `── 1. Error ('test-beta-mean.R:20:7'): ...`.
testthat's `find_srcref` walks `sys.calls()` for the first call with a
`srcref` attribute. robigo had parse-side srcrefs (expression vectors,
closure attributes, `{`-block lists all match GNU R) but never made them
visible at runtime. Ported the GNU R chain:

- `eval.c:do_eval` — each statement of an expression vector runs under
  its own srcref (`getSrcref` validation included);
- `eval.c:R_execClosure` + `context.c:begincontext` — frames save the
  srcref active at the call, closure entry switches to the closure's
  own (`interp.srcref` mirrors R's global `R_Srcref`);
- `context.c:getCallWithSrcref` — `sys.call()`/`sys.calls()` attach the
  frame's saved srcref to a copy of the call;
- `eval.c:do_begin` — `{` statements carry per-statement srcrefs (the
  block's list starts with the block itself), reset to none afterwards;
  in robigo's VM that is a new `SetSrcref` op emitted per block
  statement (a no-op in JIT regions and inlined bodies, which make no
  calls that could observe it).

Proof: new conformance `tests/r/lang/syscalls-srcref.R` (source/eval
statement srcrefs, nested closures, `{` blocks, top level) recorded from
GNU R 4.5.0 — byte-identical under robigo in interpreter, VM and VM+JIT
tiers; `tests/r/lang` and `tests/r/env` conformance 9+9 pass; fastr
`builtins/TestBuiltin_sys*` 101/101, 0 regressions; robigo-runtime lib
tests 83/83; `test_file(test-beta-mean.R)` under robigo is
byte-identical to GNU R, failure location included; the representative
use script stays byte-identical.

## Blockers (filed)

- #167: SIGSEGV in the capi bridge (`from_sexp`/`attrs_from`) converting
  a native routine's result during a real `brms` fit — every
  `brm()`-calling test file stops there. Needs the bridge fix, not a
  bayesfam change.
- #170: no message translation (LC_MESSAGES) — `LANG=de_DE` diverges in
  every error/warning text. Design decision (locale model), not scoped
  here.
- #155/#168: grDevices/grid and tools natives are not built by default;
  `scripts/build-natives.sh` needs a libcurl dev prefix on this machine.
  PR #184 improves the resulting error message.

## Reproducible commands

```sh
cd /home/m0hawk/sandbox/bayesfam
LC_ALL=C.UTF-8 Rscript tests/bayesfam-use.R            # GNU R side
LC_ALL=C.UTF-8 ROBIGO_NATIVES=<natives> robigo tests/bayesfam-use.R
LC_ALL=C.UTF-8 Rscript -e 'library(testthat); library(bayesfam);
  test_file("tests/testthat/test-beta-mean.R", reporter="summary")'
# same with robigo as the binary; diff the transcripts
```
