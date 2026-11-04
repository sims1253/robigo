# perf-nbody/superindex

Part of the perf-nbody track: the metric, the starting numbers and the
profiles are in `docs/ledger/perf-nbody.md`.

## 2026-11-04: element `<<-` in native loops, and short hot loops compile

- Why: `nbody_naive`'s loops assign `body_vx[[i]] <<- ...`, which the loop
  JIT rejected (66% `run_with`). Its loops also run 4-5 iterations per
  entry and the JIT counted back edges per entry (16), so they could never
  compile even once supported (the whole `advance` cannot run natively:
  it calls `matrix()` and `min()`).
- What (`jit.rs`, `vm.rs` `jit_enter`, `super_vec_view`, `loop_heat`;
  stacked on #294): `Op::SuperSetIndex` compiles like `SetIndex`, on a
  view of the vector of the binding `<<-` assigns, made unshared in place
  at entry (the frame must have no binding of its own and nothing may
  assign one; the frame's cached copy is dropped first, else the view
  would copy). Entries that end below the threshold add their back edges
  to `JitSlot::Heat`; a loop entered at `JIT_HEAT` compiles.
- Rejected: `JIT_HEAT = 16` (the per-entry threshold). `robigo/nbody`
  +25.7%: its inner loops compiled one by one before the outer loop did;
  `pidigits` +1.1%, `robigo/mandelbrot` +1.0%: compiles that never repaid
  (a Cranelift compile costs as much as thousands of VM iterations). At
  1024 all three are neutral.
- Result: instructions `nbody_naive` -24.3% (gate with #294: 4 faster,
  0 slower; #294's own `superWhile` -56.9%, and `fasta_naive` -6.3%,
  `fannkuchredux_naive` -3.7% from the heat); peak `nbody_naive` 0.0229 ->
  0.0083 s (Ř 0.0049, GNU R 0.0423). What is left there is entering
  native code six times per `advance` call (`jit_enter`, guards, global
  lookups): its outer loop calls `min()`.
- Verified: `tests/r/jit/loop-superassign-index.R` (global and enclosing
  vectors read back in the loop, aliases, integers and `[`, NA values and
  indices, growing the vector, overflow, a local shadow, a missing target,
  locked and active bindings, a short loop whose types change), recorded
  with GNU R 4.5.0, byte-identical in all three tiers. Conformance: as
  main.
- Review (sol-loop, gpt-6.1-sol, 1 round): approved with one nit, fixed:
  with `JIT_HEAT` at 1024 the test's short loops no longer reached native
  code; they now run past it (the JIT trace shows them compiled, and a
  deopt).
