# perf-numeric-loops: checks at native loop entry, once per environment

Stacked on the complex-loops PR (#385). Metric as in
`docs/ledger/perf-numeric-loops.md`.

## 2026-11-04: awf/Mandelbrot 38 -> 31.7 us (FastR 33.2)

- Why: awf/Mandelbrot's pixel loop is native and entered once per row (14
  times per call, its row loop stays in the VM). `jit_enter` was 12.7% of
  the time and `guard_ok_slow` 5%: every entry checked each of the loop's
  ~40 guards, resolved each inlined call's target, and checked the
  operators of the calls' arithmetic arguments.
- What (`vm.rs` jit_enter and `frames_key`, `JitLoop::checked`): those
  checks are skipped while the search epoch and the (id, function-binding
  version) of every frame from the loop's environment up to the search
  path (at most three) are those they last passed for: a function lookup
  from the loop depends on nothing else (the versions `guard_ok` and the
  inline runner rely on). Still at every entry: pending
  assignments to a called function's variable, `seq(length = n)` method
  checks, and the inlined callee's operators, which come from the callee's
  own environment (a key on the loop's environment does not cover it).
- Rejected, found by sol-loop: memoizing the callee's operators too, when
  its environment is locked down to the search path. A locked environment
  can still have its bindings replaced; `tests/r/jit/loop-entry-checks.R`
  pins the case (495 where GNU R gives 765).
- Result, peak: awf/Mandelbrot 38.1 -> 31.7 us (FastR 33.2, Ř 153),
  mandelbrot_naive_ascii 3.03 -> 2.77 ms; cycles of 20,000 awf calls -16%.
  Instructions against main: 14 faster, 0 slower (as #385; this adds no
  instructions in a single run).
- Verified: `tests/r/jit/loop-entry-checks.R` (helper rebound globally,
  `+` rebound in the loop's frame between entries, helpers in frames,
  a helper's operators replaced in its locked environment, a helper
  rebound with `<<-` in an enclosing frame), recorded with
  GNU R 4.5.0, byte-identical in all tiers; full conformance (this VM's
  LAPACK digits only, as on main); fastr loops/functions/S3/eval slice:
  same 37 failures as main (stats natives on this VM), no others; sol-loop
  3 rounds, approved.
