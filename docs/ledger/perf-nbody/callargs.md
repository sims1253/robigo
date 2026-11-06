# perf-nbody/callargs

Part of the perf-nbody track: the metric, the starting numbers and the
profiles are in `docs/ledger/perf-nbody.md`.

## 2026-11-04: inlined calls in native loops take arithmetic arguments

- What (`vm.rs` `inline_call`, `jit_enter`; `jit/closure.rs`
  `index_arg`): the call site's `inline_args` (`a op b`, `x[[i]]`, `x[i]`
  of the caller's variables) become native `CallArg`s, as whole native
  closures already had. Native code evaluates them where their fast paths
  hold and leaves at the call otherwise; `jit_enter` checks their
  functions (`-`, `[[`) are base's from the caller's environment, with the
  call site's guard memo. `index_arg` read a loop's vector through the
  scalar-slot map (a panic the new test found); outside whole closures it
  reads the view directly.
- Stacked on #296 (another instance of an inlined closure, made by each
  call of `spectralnorm_naive`): without it the loop compiles for the
  first instance only.
- Result: `spectralnorm_naive` instructions -73.2% (gate against main:
  1 faster, 0 slower, worst +0.05%); 20 runs in one process 0.77 s ->
  0.08 s; peak 0.0335 -> 0.00077 s (Ř 0.0038).
- Verified: `tests/r/jit/loop-inline-call-args.R` (integer and double
  arithmetic arguments, `[[`/`[` reads, a vector changing in the loop,
  integer overflow with its warnings, an unused argument that would fail,
  out-of-bounds reads, NA operands, `-` and `[[` rebound locally and `*`
  globally, non-numeric vectors), recorded with GNU R 4.5.0,
  byte-identical in all three tiers. Conformance: as main (on this
  machine `linalg/svd.R` and `linalg/battery.R` differ in the last digit
  on main too: robigo uses the system's `liblapack.so.3`).
