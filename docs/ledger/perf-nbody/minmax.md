# perf-nbody/minmax

Part of the perf-nbody track: the metric, the starting numbers and the
profiles are in `docs/ledger/perf-nbody.md`.

## 2026-11-04: `min()`/`max()` of two scalars in native loops

- Why: after element `<<-`, `nbody_naive`'s middle loop stayed in the VM
  for `j_from <- min(i + 1, n_bodies)`, so its inner loop was entered
  natively four times per `advance` call.
- What (`jit.rs` `min_max`, stacked on #327): `CallBuiltin` of `min`/`max`
  with two untagged integer or double scalars compiles (R:
  summary.c:do_summary, `rmin`/`imin`: integer for two integers, else
  double; of equal values the first, `x < s` replaces; NA and NaN leave
  native code). `Inl` carries the loop's code for that. `jit_enter` keeps
  16 views on the stack (the middle loop has seven; 4 spilled to the heap
  on every entry) and checks a vector's type without cloning it.
- Result: peak `nbody_naive` 0.0083 -> 0.0048 s (Ř 0.00494, GNU R 0.0423);
  instructions against main (with #294, #327) -26.3%, 0 slower.
- What is left there: `matrix(0, 5, 5)` three times per call (619 ns
  each, GNU R 632: generic closure-call cost, 9.7k instructions, no hot
  spot), and entering native code three times per call (guards, global
  lookups).
- Gap found: GNU R's compiler keeps one constant per `identical()` value,
  so in compiled code a literal `-0` becomes the `0` already in the pool
  (`min(-0, 0)` in a compiled loop is `0`); robigo keeps `-0`.
  `tests/r/basics/compiled-zero-constants.R`, no ratchet marker.
- Verified: `tests/r/jit/loop-min-max.R` (integer/double/mixed, result
  types, signed zeros passed in, NA, NaN, NA_integer_, ±Inf, logical and
  character arguments, NaN and NA together, three arguments and `na.rm`,
  a rebound `min`), recorded with GNU R 4.5.0, byte-identical in all
  three tiers. Conformance: as main.
- Review (sol-loop, gpt-6.1-sol, 1 round): approved with one nit, fixed:
  a loop compiled before `.primTrace(min)` stopped printing `trace: min`.
  Such loops now stay in the VM while a primitive is traced (as whole
  native closures do). Proved by hand, not in `tests/r`: robigo's
  interpreter tier prints `trace: min(i, 7)` where compiled code (GNU R's
  and robigo's) prints `trace: min`, the gap the perf-vm ledger records;
  20 `trace: min` lines like GNU R, 0 before.
