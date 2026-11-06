# perf-simple-loops: `for` loops in closed form as native code (round 5)

- Why: with #324 and #390, emptyFor/scalarFor cost ~1750 instructions a
  call, ~250 of them the closure tier's call of the closed form's Rust
  helper (`h_summary` -> `Summary::run`: spill the scalars to a buffer, a
  generic evaluator, reload) and ~68 the context block's
  `SmallVec::from_elem`.
- Changes:
  - `Summary::emit_for` / `emit_node` (`jit/summary.rs`): in closures
    compiled whole, a `for` loop's closed form is Cranelift code: trip
    count from the range and index, induction variables with `run`'s
    exactness checks, values at the last iteration (the start's own bits
    at `t == 1`, for -0), the last-iteration expressions with the native
    ops; the native loop where `run` would decline. `while` loops keep
    the helper (their trip count needs ceil/floor and divisions).
  - `closure::run`: the context block on the stack when it fits.
- Found by sol-loop: the exactness check used `iabs`, which lets
  `i64::MIN` through, so with ~2^32 iterations and a step of 2^31,
  `t * step` could wrap. Now `t <= 2^54 / |step|` first (the step is a
  constant), then direct comparisons with +/-2^53. No R-level test is
  possible: the repro needs ~4e9 iterations in every tier where the
  closed form declines.
- Result (peak): emptyFor 9.6e-8 -> 8.4e-8 s, scalarFor 9.6e-8 -> 8.5e-8;
  with #390 as well, 7.15e-8 for both (Ř 6.9e-8 / 7.2e-8). `perf.py
  compare` against #324: 8 faster (profiler-shared -3.0%,
  spectralnorm_alt_3 -1.7%, fastaredux -1.2%, the fasta family), 0 slower.
- Verified: `tests/r/jit/loop-closed-form.R` gains closure-tier `for` loops
  at the edges (-0, 2^53, non-integer starts, integer overflow with its
  warning), GNU R 4.5.0, identical in all tiers and with closed forms
  off; conformance; fastr seq/for/arithmetic slice; sol-loop 2 rounds,
  approved.
