# perf-nbody/freevars

Part of the perf-nbody track: the metric, the starting numbers and the
profiles are in `docs/ledger/perf-nbody.md`.

## 2026-11-04: native loops inline closures that read their environment

- Why: `spectralnorm_alt_3`'s `eval_A <- function(i, j) eval_A_cache[[i, j]]`
  reads a free variable, and an element of it: the VM's inline analysis
  admits neither, so its two loops called it through the VM
  (`native_site_call`, the closure compiled whole) 149k times per run.
- What (`vm.rs` `inline_call`, `loop_inline_info`, `jit_enter`; `jit.rs`
  `InlineCall::free`, `callee_flow`, `inline_body`): for native loops only,
  a closure's body may also read elements of vectors and matrices
  (`Index`, `Index2`) and plain scalars of its environment. They are read
  at the loop's entry from the instance each call site calls (#296), as
  values or read-only views held for the call; the callee's slots get
  their own variables in `inline_body`. Rejected at compile time if the
  loop assigns a variable of that name (it could be the binding); at entry
  if the closure is defined in the loop's own frame (its pending variables
  are not in the environment), or a value's type differs.
- Result: `spectralnorm_alt_3` instructions -56.5% (gate against main,
  with #313: 2 faster, 0 slower); 20 runs 0.62 -> 0.08 s; peak 0.0166 ->
  0.00049 s (Ř 0.00295, GNU R 0.0495).
- Verified: `tests/r/jit/loop-inline-free-vars.R` (free matrices read by
  two indices and by one, an integer vector, scalars, NA elements, out of
  range, non-numeric and classed values, a vector the loop changes, a
  removed binding, instances seeing different values, `[[` rebound where
  the closure lives, a closure in the loop's own frame), recorded with GNU
  R 4.5.0, byte-identical in all three tiers. Conformance: as main.
- Review (sol-loop, gpt-6.1-sol): round 1 found a blocker. The identity
  check rejected a closure defined in the loop's own frame but not one
  whose environment only reaches that frame further up (`local(function(i)
  x + i)`): it read `x` from the environment while the loop's newer value
  was still pending in the VM (15450 instead of GNU R's 15750). Both
  checks now use `closure::on_chain`; the case is in the test.
- Found by the stacked conformance run: with #294 underneath (native
  `<<-`), a loop that assigns the free variable with `<<-`
  (`lang/compiled-jit.R` `callfree`, `k <<- 1000` mid-loop) kept the value
  read at entry. `<<-` writes of the name now reject the loop too; the
  case is in the test.
- Gap (interpreter tier only, so not a `tests/r` script): inside a
  function, `x <- 1; f <- local(function(i) x + i); for (j in 1:2) f(j)`
  fails with "attempt to apply non-function" when not compiled
  (`ROBIGO_COMPILE=0`); the VM runs it. The test builds the closure
  without `local()`.
