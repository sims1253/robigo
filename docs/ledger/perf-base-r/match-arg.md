# perf-base-r / match-arg: `match.arg` without running its R code

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
- Result (release, `rkparts.R`, against main): `match.arg` 4.64 -> 0.17
  µs, `sort.list` 12.7 -> 4.9 µs, `order(x, y)` 10.2 -> 6.7 µs,
  `rank(random)` 36.1 -> 17.4 µs (local GNU R 4.5.0: 23.2). Peak
  (`rivals.py`): `flexclust_kcca` 4.43 -> 2.12 s (GNU R 4.12, Ř 5.45),
  `sortorder` 0.025 -> 0.022 s (GNU R 0.026). Instructions against main:
  4 faster (`flexclust_kcca` -29.6%, `csv` -2.0%, `sortorder` -1.6%,
  `mandelbrot_ascii` -0.5%), 0 slower.
- Cheaper on the way: the caller's unforced default is filled with the
  choices just read instead of running `c()` through the VM (-2.7k
  instructions per `order` call), and the base-binding check is memoized
  on the search epoch.
- Found by the gate: `robigo/csv` measured +15.8% under `perf.py` (from a
  fresh temporary directory; +0.7% elsewhere). Not this change's code:
  `Scan::extract` allocated and freed a String per field, and in that heap
  state mimalloc took and returned a fresh page for those blocks over and
  over (`mi_page_free_list_extend` +43M). The field is now borrowed unless
  it needs repair: csv -2.0% and no longer sensitive to the heap.
- Verified: `tests/r/lang/match-arg-fast.R` (exact/partial/ambiguous/NULL,
  every error and its call, attributes, a promise with side effects, a
  frame-local and a global `c`, a user `match.arg`, a symbol that is not a
  formal of the caller, S3 methods, `eval()`, non-ASCII choices, tagged
  and computed defaults), identical in all tiers.
- Sol review (round 1) found a traced helper unobserved:
  `trace(pmatch)` prints `trace: pmatch` in GNU R on every match.arg
  call, and the fast path skipped it. The base-binding memo (recomputed
  when the search epoch changes, which `trace()` does) now also requires
  `formals`, `sys.function`, `sys.parent`, `sys.frame`, `eval`,
  `identical` and `pmatch` to be base's plain, untraced closures; the
  case is in `tests/r/lang/match-arg-fast.R`.
