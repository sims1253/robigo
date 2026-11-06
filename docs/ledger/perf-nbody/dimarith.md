# perf-nbody/dimarith

Part of the perf-nbody track: the metric, the starting numbers and the
profiles are in `docs/ledger/perf-nbody.md`.

## 2026-11-04: arithmetic on `dim`-only double arrays without the flush

- Why: per-op timings of `nbody_2`'s `advance` against GNU R: `dt / (d *
  d * d)` on a 5x5 matrix 830 ns (GNU R 520), `body_r <<- body_r + dt *
  body_v` 505 ns (GNU R 260). Arrays carry a `dim` attribute, so their
  arithmetic still took the flush-and-builtin path.
- What (`vm/slow.rs` `Shape::dims`, `pure_plan`; `builtins/arith.rs`
  `plain_real_arith`; stacked on #319): doubles whose only attribute is
  `dim` (rank at most 4) take the same path where R keeps the extents
  unchanged (R: arithmetic.c:R_binary): two arrays with equal extents, or
  an array with a vector of its length or a scalar. Recycling, a length-1
  array with a longer vector (a deprecation warning) and mismatched
  extents (an error) still take the builtin.
- Result: `dt / (d * d * d)` 830 -> 305 ns, `br + dt * bv` 505 -> 195 ns.
  Gate against main (with #319): `rbench/nbody` -4.1%, `nbody_2`/`_3`/
  `_naive_2` about -1.6% each, 0 slower.
- Still open on `nbody_2` (peak 0.0193 with everything, Ř 0.0165): base
  R's `colSums` (3.9 us per call, GNU R 2.3) and `diag<-` (3.8 us, GNU R
  1.8), 42k instructions per `colSums` call spread over the VM running
  its body, partly through the AST interpreter. Generic base-closure
  execution (group 8's area), not done here.
- Verified: `tests/r/vec/dim-double-arith.R` (every operator, conformable
  and non-conformable arrays, arrays with scalars and same-length vectors
  both ways round, recycling, the length-1-array warning, 3-d arrays,
  dimnames, zero extents, NA/NaN payloads, copies, accumulation and
  multiply-add chains, `tsp` and classed vectors), recorded with GNU R
  4.5.0, byte-identical in all three tiers. Conformance: as main.

## 2026-11-04: rejected and open

- Rejected: hashing the compact-sequence registry's addresses without
  SipHash (`value.rs` `Registry`). `seq_len(5L)` 2181 -> 2031 instructions
  (-7%), but no benchmark moved by more than 0.42% (geomean -0.01%).
- Open, measured: robigo's builtin calls cost about twice GNU R's
  (`seq_len(5)` 115 vs 45 ns, `dn[-id]` 240 vs 105, `dim(z) <- dn` 225 vs
  110, `x[m] <- 0` 415 vs 100, `is.data.frame(x)` 395 vs 240). Of
  `seq_len(5)`'s 2.2k instructions, ~500 are the flush `Op::CallBuiltin`
  does first (`Vars::flush`, then `set_var`/`binding_flags` writing the
  locals back). It is conservative because a builtin may signal, and a
  calling handler could inspect the frame. Measured upper bound of a lazy
  flush (dropping it, not shippable): `nbody_2` only -3%, so rejected for
  now. `nbody_2`'s remaining gap (peak 0.0174 with every PR here, Ř
  0.0165) is base-closure execution spread flat: `colSums` 3.6 us (GNU R
  2.3), `diag<-` 3.8 us (GNU R 1.8), 32-42k instructions per call with no
  hot spot (argument matching, promises, frames, complex assignment).
