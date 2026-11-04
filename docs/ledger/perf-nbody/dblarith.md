# perf-nbody/dblarith

Part of the perf-nbody track: the metric, the starting numbers and the
profiles are in `docs/ledger/perf-nbody.md`.

## 2026-11-04: arithmetic on plain double vectors and complex scalars without the flush

- Why: after the slices, `x[, i] - x[, j]` was still 332 ns: `a - b` on two
  length-3 doubles took 259 ns (GNU R 43). The VM's slow path flushed every
  pending local to the environment (`Vars::flush` 14%), called the builtin
  through `invoke_builtin`, then looked the locals up again
  (`plain_lookup` 13%).
- What (`vm.rs` `Op::Arith`/`ArithL`, `vm/slow.rs`, `builtins/arith.rs`
  `plain_real_arith`): operands are classified without running code or
  cloning (`Shape`, `pure_plan`). Attribute-free doubles that recycle
  without a warning (equal lengths, or one of length 0 or 1), op not `%%`,
  compute before any flush through the builtin's own kernels (in-place
  reuse of temporaries, NaN payload rules) and its fused multiply-add
  pass; complex scalars through `complex_op`, as the slow path did. Neither
  signals or runs code.
- Rejected on the way: the first check cloned a variable's value before
  rejecting it, on every slow-path `ArithL`; `mandelbrot_naive_ascii` was
  +4.9% (complex scalars), then +2.6% with the clone removed (80
  instructions per rejected call). Taking complex scalars through the
  same path instead made it -15%; stopping at the first operand that
  cannot qualify took `mandelbrot_ascii`/`_noout` (complex vectors) from
  +0.64% to neutral.
- Result: `a - b` (length 3) 259 -> ~75 ns. Gate against main: 2 faster
  (`mandelbrot_naive_ascii` -15.3%, `nbody_naive_2` -1.9%), 0 slower. On
  top of the slices, 20 runs of `nbody_2` 0.72 -> 0.42 s.
- Verified: `tests/r/vec/plain-double-arith.R` (every operator, NA/NaN
  payloads both ways round, division by zero, lengths 0/1/n, recycling
  with and without its warning, `%%` and its warning, integer, logical,
  named and matrix operands, copies, in-place temporaries, multiply-add
  chains, a rebound `+`, complex scalars with NA/NaN and the `%%`/`%/%`
  errors, complex vectors), recorded with GNU R 4.5.0, byte-identical in
  all three tiers. Conformance: as main.
