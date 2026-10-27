# numerics ledger

Software 80-bit long double everywhere, its speed, and the exact/fast
numeric-policy seam. Metric: (1) bit agreement with x87 over property
tests, operations covered; (2) instructions per operation; (3) policy
kernels implemented and benchmarked.

## 2026-10-27: software long double core, verified against the FPU

- why: exact-mode numerics silently fell back to `f64` off x86_64, so
  `sum`, `cumsum`, `var`, ... produced different bits than GNU R.
- what: `ldouble/soft.rs`, a software x87-semantics 80-bit type (add,
  sub, mul, div, sqrt, rint, cmp, from_f64/from_i64/to_f64, the loop
  kernels); non-x86_64 `imp` uses it. Property tests on x86_64 compare
  every operation against the x87 asm path bit for bit (1M random
  inputs per generator + edge pairs + alignment sweep + NaN rules;
  `ROBIGO_LDPROP_N` tunable).
- result: before — no software path (off-x86_64 results simply wrong);
  after — 29/29 tests pass, 0 mismatches over ~2·10⁷ comparisons
  (6.8 s release). Mutation check: flipping the round-to-even tie in
  `pack` fails `arithmetic_matches_x87` immediately, so the oracle
  wiring is live. kept.
- verification of the inherited WIP: the previous agent's branch was
  unverified; re-reviewed `soft.rs` (NaN selection, denormal single
  rounding, the `de == 65` add shortcut), rebased on current `main`,
  mutation-tested, then rewritten into the two commits above.

## Open

- `soft::powl`: port glibc 2.41 x86_64 `powl` (e_powl.S special cases
  + binary-powering integer path + `__powl_helper`'s table/series
  algorithm) including software `f2xm1`, `fyl2xp1`, `fscale` that
  match the hardware bits; property test vs `imp::powl`.
- cross builds: `cargo build -p robigo-numerics --target
  aarch64-unknown-linux-gnu` and `wasm32-unknown-unknown` (blocked
  only on `soft::powl`).
- callgrind measurement of the software ops; make them fast.
- numeric-policy seam (exact/fast) for the ldouble kernels + benchmark.
