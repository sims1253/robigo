## 2026-11-14: X1/X2, the decision bars' rows at the base `e2207b4d`

The runs of the default variants that every X1/X2 decision compares
against (docs/ledger/g3.md, "M1 decision bars"; m1-plan §B X1, X2). Each
decision's run entry cites this one.

- Base: main `e2207b4d386a4e00d875955af33b203e7fd5beec` (V2 #664 merged).
  On it, every default of the bars: `inline` frames with the overflow area
  (N1b), 16-byte value-stack slots (R1), delayed/forced argument slots
  (C1), no frame recycling, deferred link counts (C1, V1b), dead registers
  cleared at last use (V1a).
- Binary: `robigo-rt`, `cargo build --release --locked -p robigo-rt` with
  a private `CARGO_TARGET_DIR` (foreman's notes: shared-target clobbering),
  copied to `~/.cache/x1-bins/robigo-rt-base-e2207b4d` before measuring.
- Machine: WSL2 workstation, AMD Ryzen 9 5900X (24 threads), Linux
  6.18.58.1-microsoft-standard-WSL2, rustc 1.100.0, valgrind 3.27.1 (the
  one `scripts/perf.py` picks, `~/.cache/rr-tools/bin/valgrind`). Every run
  of X1/X2 is on this machine.
- Harness (added in this PR):
  - `scripts/idiom-vs-r.py --rows ... --allocs`: allocations per iteration,
    objects and bytes by SEXPTYPE from the heap's `ROBIGO_GC_STATS=1`
    report, N against 2N (the bars' allocation metric; no callgrind).
  - `scripts/rt-bars.sh BIN OUTDIR`: the CALLS and SUITE rows under
    callgrind with `--cycles` (instructions and estimated cycles, N against
    2N), as four groups in parallel (the machine's limit of four callgrind
    runs), then `--allocs`. The bars run every table twice: it is called
    twice per binary.
  - `scripts/rt-bars-compare.py --a NAME DIRS --b NAME DIRS --rows ...`:
    the reproduction check (spread of the runs, flagged over 0.1%
    instructions or 0.5% estimated cycles), medians, b ÷ a per row and
    their geomean over the bar's rows, and the allocation table.
- Commands:

  ```sh
  scripts/rt-bars.sh ~/.cache/x1-bins/robigo-rt-base-e2207b4d RUN1
  scripts/rt-bars.sh ~/.cache/x1-bins/robigo-rt-base-e2207b4d RUN2
  ```

- Raw JSON: `data/2026-11-14-x-base-e2207b4d/run{1,2}-{plain,m1,fib,bt,allocs}.json`.
- Admission: `scripts/rt-ci.py` at the base passes (exit 0): release tests,
  conformance 201/207 (the 6 failures are the unratcheted
  `args/binding-types`, `env/binding-errors`, `env/closure-frames`,
  `env/hashed-env`, `env/unhashed-env`, `jit/setter-calls`), gctorture,
  `ROBIGO_MATERIALIZE=1`, `ROBIGO_GC_MOVE=1`, `ROBIGO_GC_PIN_REPORT`.
- Rows that run at the base: CALLS (`plain`, `defaults`, `named`, `env-fn`,
  `superassign`, `s3`) and SUITE (`fib`, `binarytrees`) all run. Of D2S,
  only `d2s-eval` runs: `d2s-assign` and `d2s-callee-assign` stop at
  "could not find function \"assign\"", `d2s-delayed` at "could not find
  function \"delayedAssign\"". The loader binds only the base closures
  that the M1 scripts reach (`scripts/base-subset.sh`), and the D2S rows
  are not among those scripts. Until they are bound, D2s's runs are
  preliminary and decide nothing (bar protocol). D2s is decided after D2,
  so this blocks nothing yet.

Reproduction (`scripts/rt-bars-compare.py`; spread = max ÷ min − 1 of the
two runs). Instructions and estimated cycles per iteration:

| row | run 1 instr | run 1 est. cycles | run 2 instr | run 2 est. cycles | instr spread | cycles spread |
|---|---:|---:|---:|---:|---:|---:|
| plain | 6,908.0 | 7,383.1 | 6,908.0 | 7,383.0 | 0.000% | 0.000% |
| defaults | 8,579.5 | 9,145.3 | 8,579.4 | 9,145.2 | 0.000% | 0.001% |
| named | 6,619.2 | 7,094.2 | 6,619.3 | 7,094.3 | 0.002% | 0.001% |
| env-fn | 7,309.1 | 7,765.6 | 7,309.1 | 7,765.6 | 0.001% | 0.001% |
| superassign | 10,707.8 | 11,501.1 | 10,707.8 | 11,501.3 | 0.001% | 0.002% |
| s3 | 32,380.3 | 37,541.6 | 32,380.3 | 37,584.8 | 0.000% | 0.115% |
| fib | 44,749,424.0 | 46,122,072.5 | 44,749,408.0 | 45,952,095.5 | 0.000% | 0.370% |
| binarytrees | 8,227,018.3 | 8,753,046.3 | 8,227,030.7 | 8,748,954.5 | 0.000% | 0.047% |

No row is over the thresholds (0.1% instructions, 0.5% estimated cycles),
so two runs suffice. Estimated cycles move more than instructions (fib
0.37%): the cache simulation sees addresses, which ASLR changes.

Allocations per iteration (identical in both runs):

| row | objects | bytes | by type (objects) |
|---|---:|---:|---|
| plain | 3 | 228 | ENVSXP 1, REALSXP 2 |
| defaults | 3 | 260 | ENVSXP 1, REALSXP 2 |
| named | 2 | 212 | ENVSXP 1, REALSXP 1 |
| env-fn | 3 | 196 | ENVSXP 1, REALSXP 1, STRSXP 1 |
| superassign | 5 | 292 | ENVSXP 1, LISTSXP 1, REALSXP 2, STRSXP 1 |
| s3 | 27 | 1,892 | ENVSXP 5, LANGSXP 1, LISTSXP 12, PROMSXP 4, REALSXP 1, STRSXP 4 |
| fib | 20,901 | 1,404,580 | ENVSXP 8,361, REALSXP 12,540 |
| binarytrees | 2,809 | 183,892 | ENVSXP 1,022, LGLSXP 511, REALSXP 765, VECSXP 511 |
