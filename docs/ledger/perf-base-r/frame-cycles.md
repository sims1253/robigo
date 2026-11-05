# perf-base-r / frame-cycles: freeing frames held only by themselves

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

## 2026-11-04: free self-referencing frames at return (kept)

- Why: the env <-> promise and env <-> closure cycles above. The frame
  was handed to the cycle collector (`track_escape`), which only runs on a
  collection, which a pure R program never triggers: memory grows without
  bound, and every call pays fresh pages instead of reused ones.
- What (`env.rs` `Env::release_self_cycle`, `interp.rs` `end_context`):
  when a returning frame has references beyond the executor's and the
  context's, count those its own bindings explain: an unforced promise
  whose environment is the frame and that only the binding holds
  (strong count 1, not being forced), a closure of the frame only the
  binding holds, the environment bound in itself. If they explain every
  extra reference, nothing else reaches the frame: clear its bindings
  (what `reachability` would do at the next collection). Any reference it
  cannot attribute (a returned closure, `environment()` escaping, a
  `delayedAssign` elsewhere, a finalizer's proxy, a call-site cache
  holding the closure) leaves the frame to the collector as before.
- Result (wall, this VM): `function(x, d = c(1,2)) x` x5M 2.12 s / 2.2 GB
  -> 0.99 s / 24 MB; a frame with a local closure x2M 1.21 s / 792 MB ->
  0.43 s / 24 MB; `stopifnot(TRUE)` x300k 374 MB -> 26 MB; `rank(random)`
  x100k 4.02 s / 344 MB -> 3.70 s / 43 MB.
- CI's paired gate (startup cache on) flags `lm` +1.04%, `s4-valid` +0.71%,
  `s4-new` +0.56%. On `s4-valid` the check runs 11,173 times (1.7M
  instructions self), the rest is drops and `mi_free` of the freed frames.
  Rejected, measured: skipping the walk for frames never given a default
  promise or a closure (a flag set by `default_arg` and `function`):
  -0.1M, the frames that reach the check almost all have one; counting
  them instead, to skip frames with more outside references than inner
  ones: +0.9M (the increment on every default promise costs more than the
  walks it saves). The walks mostly succeed: the cost is the freeing.
- Verified: `tests/r/basics/frame-self-cycles.R` (defaults escaping through
  closures and `environment()`, `delayedAssign` into another env,
  `reg.finalizer` on a frame, `on.exit` and `sys.frame` during the call,
  self-bound frames), byte-identical in all three tiers.
- Peak (`rivals.py`, main and candidate measured back to back on this VM):
  `robigo/lm` 0.043 -> 0.026 s (-40%: the leak slowed every warmed-up
  iteration; GNU R 0.026), `flexclust_kcca` 4.43 -> 3.68 s (-17%; GNU R
  4.12, Ř 5.45: robigo now fastest), `sortorder` 0.025 -> 0.0255 (flat).
- Instructions (`perf.py compare`, startup included): 0 faster, 12 slower
  by more than 0.5% (worst `convolution_v` +1.0%, `lm` +0.85%, `s4-valid`
  +0.74%, the rest +0.5-0.7%). Every program pays ~1.2M more at startup:
  the check (1,888 calls, 250k) and the drops and frees of frames that
  used to leak (~950k), offset by `track_escape` and mimalloc taking fresh
  pages less often (-0.2M). Callgrind cannot see what leaking costs: the
  page faults of fresh memory. Startup in cycles (`perf stat -r 40`, two
  alternations): main 303-322M, candidate 271-286M; RSS 55 -> 53 MB.

## 2026-11-04: zlib-rs with its `std` feature: SIMD adler32 (kept, with the above)

- Why: the frame freeing above costs instructions on the S4 benchmarks
  (CI: `s4-valid` +0.71%, `s4-new` +0.56%) that no change to it removes
  (the walks succeed; the drops are the fix). Looking for what those
  programs spend: `library(methods)` is half of `s4-valid`, and zlib-rs's
  *scalar* `adler32` alone is 17.9M of it (4.2M in every startup).
  flate2 builds zlib-rs without `std`, so zlib-rs cannot detect AVX2 at
  run time and never takes its SIMD checksum.
- What (`crates/robigo-runtime/Cargo.toml`): depend on zlib-rs directly
  with `std` (and flate2's `rust-allocator`), unifying the features. The
  checksum is the same; only the implementation changes.
- Result: startup 208.5 -> 199.7M instructions (-4.2%), `s4-valid` -4.5%,
  `lm` -1.7% on its own.
