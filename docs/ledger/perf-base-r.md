# perf-base-r: base R library code on the general closure path

Benchmarks: `robigo/lm`, `robigo/sortorder`, `rbench/real/flexclust_kcca`
(`apply(m, 1, rank, ties.method = "random")` over 110k rows). Metric: peak
and shot wall time against GNU R, Ř and FastR (`scripts/rivals.py`); gate:
`scripts/perf.py compare` (no benchmark more than 0.5% slower) and
conformance in all tiers.

Baseline (main 5ce8d760, seconds, peak / shot):

| benchmark | robigo | GNU R | Ř | FastR |
|---|---|---|---|---|
| robigo/lm | 0.0315 / 0.0641 | 0.026 / 0.0947 | 0.0333 / 0.135 | 0.178 / 4.1 |
| robigo/sortorder | 0.0262 / 0.0726 | 0.026 / 0.115 | 0.027 / 0.431 | 0.398 / 4.5 |
| real/flexclust_kcca | 4.16 / 4.18 | 4.12 / 4.54 | 5.45 / 7.06 | - |

## 2026-11-04: where the time goes

- `rank(c(0.3, 0.1), ties.method = "random")` is 40-45 µs per call in
  robigo, the body of flexclust_kcca. Callgrind: 440k instructions per call
  against GNU R's 397k (1.1x), but `perf stat` shows 1.29x the cycles: 2.8x
  the branch misses and 1.9x the L1i misses. The profile is flat (VM
  dispatch 24%, then frames, promises, contexts, clone/drop, malloc).
- Per R function, the gap is the wrappers: `match.arg` (4 calls per `rank`:
  rank, sort.list, order twice) is 1.3-2 µs slower each than GNU R;
  `stopifnot`, `formals`, `eval`, `identical` smaller ones.
- Found on the way: a call whose frame is referred to by its own bindings
  (an unforced default argument's promise, a closure made in the frame)
  was never freed. 5M calls of `function(x, d = c(1, 2)) x` reached
  2.2 GB RSS (GNU R: 40 MB) and ran 2.7x slower than GNU R; `stopifnot`
  (local closures) 374 MB after 300k calls, `rank` 344 MB after 100k.

## 2026-11-04: `match.arg(arg)` without running it (kept)

- Why: with the leak fixed, `rank(x, ties.method = "random")` was 33.6 µs
  against the local GNU R 4.5.0's 20.8. `match.arg` runs four times per
  `rank` (rank, order twice, sort.list) at 4.4 µs each in robigo (GNU R
  2.8): five closure calls (`sys.parent`, `sys.function`, `formals`,
  `eval`, `pmatch`...) and ~40 VM ops to read the caller's default
  `c("a", "b", ...)` and partially match one string against it. Slicing
  match.arg into prefixes showed the time evenly spread: no single
  operation to fix, the whole function is the cost.
- What (`intrinsic.rs`, a `CallSite::intrinsic` set by the compiler for
  `match.arg(<symbol>)`): when the callee is base's own `match.arg`
  (untraced, no traced primitive), the outermost function frame of the
  calling environment is a closure whose formal of that name defaults to
  an untagged `c()` of ASCII strings, `c` resolves to base's from there
  (the VM's guard memo), and the argument is a value, a forced promise or
  its own unforced default (forced as usual), the value is computed
  directly: NULL -> first choice, identical -> first, else exact then
  unique partial match. Everything else, including every error, is the
  real call. Interpreter tier untouched (same answers, slower).
- Result (release, `rkparts.R`): `match.arg` 4.40 -> 0.19 µs, `sort.list`
  12.3 -> 5.3 µs, `order(x, y)` 9.9 -> 7.2 µs, `rank(random)` 33.6 ->
  17.6 µs. Peak `flexclust_kcca` 3.68 -> 2.26 s (GNU R 4.12, Ř 5.45).
- Verified: `tests/r/lang/match-arg-fast.R` (exact/partial/ambiguous/NULL,
  every error and its call, attributes, a promise with side effects, a
  frame-local and a global `c`, a user `match.arg`, a symbol that is not a
  formal of the caller, S3 methods, `eval()`, non-ASCII choices, tagged
  and computed defaults), identical in all tiers.
