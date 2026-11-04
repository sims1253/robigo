# perf-base-r: base R library code on the general closure path

Benchmarks: `robigo/lm`, `robigo/sortorder`, `rbench/real/flexclust_kcca`
(`apply(m, 1, rank, ties.method = "random")` over 110k rows). Metric: peak
and shot wall time against GNU R, Ř and FastR (`scripts/rivals.py`); gate:
`scripts/perf.py compare` (no benchmark more than 0.5% slower) and
conformance in all tiers.

## 2026-11-04: guards of closures made anew per call, memoized on the enclosure (kept)

- Why: `order()` builds `function(x) (is.numeric(x) || ...)` for `vapply`
  on every call. The VM skips a closure's guards once they are known to
  hold for its environment (`fresh_nested_ok`), but checks them only from
  the second call with the same environment on: a closure made per call
  and called twice recomputed all 8 guards each time (~2.1k
  instructions, `resolves_to_base`) and ran its first call with guards.
- What (`vm.rs` `fresh_nested_ok`, a `Code::fresh_outer` memo): for a
  closure whose environment is a call frame whose enclosure is on the
  search path or locked (a namespace), the guards depend only on that
  frame's own function bindings (checked per call, `local_may_bind_fun`
  for each guarded name) and the enclosure (search epoch, its id and
  function-binding version): memoized on those. It also sets
  `fresh_nested`, which `eager_args` reads (without it `queens` was +3.3%,
  `profiler-microbenchmark` +1.8%: callers fell back to promises).
- Result: `order(x, y)` -6.3k instructions per call. Against main: 2
  faster (`pidigits` -1.0%, `flexclust_kcca` -0.9%), 0 slower.
- Verified: `tests/r/lang/fresh-closure-guards.R` (a frame that binds a
  guarded name or binds it later in the call, a global rebinding between
  calls, a formal named like a guarded function holding a non-function),
  identical in all tiers.
