## 2026-11-16: T1, determinism, overshoot and allocation failure (E3, E4, E9) on robigo-rt

- What: M1's collection criteria (docs/g3.md §15.3 E3, E4, E9; §3 items
  3-5) measured on robigo-rt, with the tools that make them repeatable and
  CI steps that keep them (m1-plan §B T1). D1's run and decision, which
  cite these, are `2026-11-16-d1-run.md`.
- Tree: branch `g3/t1-collection` at `c076b56a` (main `7af7704e` and T1's
  commits). Binary: `cargo build --release --locked -p robigo-rtapi`
  (`robigo-rt`), private target directory. Machine: AMD Ryzen 9 5900X (24
  threads), WSL2 Linux 6.18, `kernel.randomize_va_space` 2, rustc 1.100.0.
  GNU R: the 4.6.2 oracle image for every recording,
  `~/.cache/robigo-research/R-4.6.2-build` for peak RSS.

### E3: determinism

- **The stress script** (`tests/rt/stress/finalizer-depths.R`: a
  finalizer-carrying pointer made at R depth a, a collection at depth b by
  C churn or `gc()`, a and b in 0..24): `scripts/rt-stress.sh --runs 30`,
  a fresh process per run under full ASLR, each profile forced with
  `ROBIGO_RT_PROFILE`. **Eval 30 of 30, Bc 30 of 30 runs missed nothing**
  (each run prints "missed 0 of 625" for both kinds).
- **weak-keys-collections** (`tests/rt/capi/`, A1's rt variant) passes and
  is ratcheted.
- **The rt slice under the collector modes** (`cargo test --release -p
  robigo-rtapi --test conformance` with `ROBIGO_CONF_TIMEOUT=600`): under
  `ROBIGO_GCTORTURE=1` and under `ROBIGO_GCTORTURE=1 ROBIGO_GC_MOVE=1` it
  prints what the plain run does: 230 of 235 in all three (the 5 failures
  fail plain too and have no ratchet marker). Two cases needed longer than the
  runner's 20 s under the modes (`tests/r/jit/loop-closed-form-super.R`,
  21 s under gctorture; the stress script, about 80 s under the moving
  collector): `ROBIGO_CONF_TIMEOUT` lets a run allow more.

### E4: overshoot and peak RSS

- **Overshoot** (`scripts/rt-overshoot.py`): 251 programs, each in both
  profiles under `ROBIGO_GC_STATS=1` (every ratcheted `tests/rt` and
  `tests/r` case, `benchmarks/suite/{calls,fib,binarytrees}.R`,
  `benchmarks/rt/mem/*.R`, and every row of `benchmarks/rt/calls-rows.toml`
  as its N loop). **No overshoot passed max(32 MiB, 10% of the live heap)
  in any of the 502 runs**, and none ended with a collection still due
  (the heap's report now gives that case: an overshoot no collection
  served would otherwise go uncounted). The largest:

  | program | profile | collections | max overshoot | live after | max / bound |
  |---|---|---:|---:|---:|---:|
  | benchmarks/rt/mem/churn.R | eval | 79 | 8,000,032 | 20,675,520 | 0.238 |
  | benchmarks/rt/mem/churn.R | bc | 79 | 8,000,032 | 24,674,832 | 0.238 |
  | tests/rt/capi/weak-keys-collections.R | both | 121 | 800,032 | 501,408 | 0.024 |
  | tests/rt/stress/finalizer-depths.R | both | 2,119 | 800,032 | 515,808 | 0.024 |

  The three `e7-*` rows exit 1 on robigo-rt (`seq.default` is not in the
  M1 subset, the foreman's R1d follow-up): their overshoot covers what ran.
- **Peak RSS** (`scripts/rss.py --runs 5`, the median `ru_maxrss`; GNU R
  4.6.2's `exec/R`):

  | program | robigo-rt KiB | GNU R 4.6.2 KiB | ratio |
  |---|---:|---:|---:|
  | benchmarks/suite/binarytrees.R | 28,904 | 43,668 | 0.66 |
  | benchmarks/rt/mem/listgrow.R | 44,520 | 55,712 | 0.80 |
  | benchmarks/rt/mem/churn.R | 72,640 | 136,008 | 0.53 |

  binarytrees.R and listgrow.R did not run on robigo-rt before this branch:
  `seq.default` needs `.Machine` (now `Init_R_Machine`, over a port of
  machar.c in robigo-numerics: `print(.Machine)` matches GNU R's), and
  `lengths()` needed its internal (`do_lengths`; a classed `x` or element,
  which GNU R reads through `[[` and `length` methods, says it is not
  implemented yet).

### E9: allocation failure

- **`R_MAX_VSIZE` was ignored**: the heap read it (`R_SizeFromEnv`), and
  the isolate then set its own default, no limit, over it. `Options`'
  `max_vsize` now overrides only when given.
- **A small vector limit**: the rt slice passes under `R_MAX_VSIZE=64M`
  (the smallest the default 64M vector heap accepts; below it GNU R's
  "too small maximum ... ignored" warning) and under `R_VSIZE=4M
  R_MAX_VSIZE=16M`: 230 of 235, as plain.
- **GNU R's messages** (`tests/rt/alloc/vector-limit.R`, recorded on
  4.6.2, through `.Internal(mem.maxVSize())`, now ported): "vector memory
  limit of 200.0 Mb reached, see mem.maxVSize()", "cannot allocate vector
  of size 7450580.6 Gb", "a limit lower than current usage, so ignored",
  verbatim.
- **Injected failure** (`ROBIGO_FAIL_ALLOC_AT=n`, a knob in the heap's
  slow path, armed after base is loaded: the script's `n`th allocation is
  refused once, as a refused mapping is, so the code that made it collects
  and retries): `scripts/rt-fail-alloc.py` compares each injected run's
  stdout, stderr and status with the uninjected run's.
  - `tests/rt/alloc/effects.R` (12 rounds of a `cat`, a binding store, an
    assignment into a vector, a warning every fourth, allocations of
    1,000 to 12,000 doubles): **all 2,252 allocations, each refused in
    turn: the output never changed**.
  - every ratcheted `tests/rt` case, 40 points spread over each one's
    allocations: **6,917 injected runs over 188 cases, none changed**.
  - That the knob fires: with `ROBIGO_GC_STATS=1` an injected run shows
    one collection of kind `retry` that the uninjected run does not.

### What CI runs now (`scripts/rt-ci.py`, the `rt` job)

After the existing steps: the conformance under gctorture and under the
moving collector, `scripts/rt-stress.sh --runs 3`, `scripts/rt-overshoot.py`,
the conformance under `R_VSIZE=4M R_MAX_VSIZE=16M`, and
`scripts/rt-fail-alloc.py --points 400`. Locally the whole `scripts/rt-ci.py`
ran in 345 s, the new steps in about 170 of them (120 the conformance under
the moving collector); every step passed.
