# perf-vector-loops: fannkuch-style integer vector loops

Metric: `scripts/rivals.py` peak (fastest warmed-up iteration) against GNU R,
Ř and FastR on `fannkuchredux`, `fannkuchredux_naive`, `vectorFor`,
`robigo/queens`, `robigo/fannkuch`. Gate: `scripts/perf.py compare` of
instruction counts against main (no benchmark more than 0.5% slower) and
conformance in all three tiers.

Baseline (main 5ce8d760, peak seconds): fannkuchredux 0.0239 (GNU R 0.0118,
Ř 0.00387, FastR 0.000488); fannkuchredux_naive 0.00522 (0.00625, 0.00133,
0.000516); vectorFor 0.00252 (0.0297, 0.00197, 0.00491); queens 0.00347;
robigo/fannkuch 0.05.

## 2026-11-04: round 1, whole fannkuch loops in native code

- Why: `fannkuchredux_naive` is scalar element swaps, which the loop JIT has,
  but only its inner loops compiled (4,400 native entries per run). The outer
  `while (TRUE)` was rejected for three reasons: `perm <- perm1` (a vector
  assigned to a variable), `max(max_flip_count, flip_count)` (a builtin call),
  and `cat(...); return(...)` at the very end (calls). Also `if (k <- perm1[[1L]])`
  (an integer condition) and `for (r in rxrange)` over a range held in a
  variable. FastR at 0.5 ms compiles the whole function.
- Change (`jit.rs`, `vm::jit_enter`), loop tier only:
  - Leaving at ops native code lacks: a call or `return` at the loop's own
    level (not in a nested loop) compiles to a deopt at that op; the VM
    executes it and goes on. The registers such an op reads must be ones
    native code restores or never writes. A loop that cannot complete an
    iteration without leaving (no back edge on a cycle of native ops) is not
    compiled, and one that keeps leaving (4 leaves, fewer than 16 back edges
    per leave) is given up: native code counts its back edges.
  - Vector variables assigned in the loop (`perm <- perm1`) are *owned*: a
    buffer of the run's `LoopCtx`, copied from the value at entry if read
    first, grown by a helper, written back as a new vector at exit and deopt.
    `GetVar` of a vector gives a vector register (pointer, length) that views
    it; a loop where such a register is live across a change of the vector it
    views is rejected, and vector registers are restored at a deopt only if
    live (liveness analysis), since a dead one may view a resized buffer.
  - Range variables (an unmaterialized `a:b`, or a compact sequence) as three
    values, readable as `for` sequences; `length()` of vectors and ranges.
  - `max`/`min` of two to four scalars (R: summary.c; NA and NaN deopt, ties
    keep the first); `if`/`while` on integer and double scalars
    (asLogical2's `x != 0`, NA deopts).
- Rejected on the way:
  - leaving at any op: startup +0.4% (two base loops, `dynGet`'s and
    `registerS3methods`', compiled and left in every iteration), the whole
    suite +0.7%;
  - "a back edge is reachable" as the test for running an iteration: a
    `while` loop's condition is reachable without its body;
  - leaving at `:` and at calls in nested loops: `robigo/fannkuch` +2.5%,
    `spectralnorm_naive` +1.1%, `spectralnorm_alt_3` +1.0%,
    `mandelbrot_naive_ascii` +1.1% (compiled, then left at once every entry).
- Result: `fannkuchredux_naive` peak 5.22 ms -> **0.41 ms** (FastR 0.52,
  Ř 1.33, GNU R 6.25); shot 48 -> 35 ms. Gate against main bf7332ee: 1 faster
  (`fannkuchredux_naive` -7.4% with startup), 0 slower; worst +0.33%
  (`robigo/nbody`), startup +0.03%.
- Verified: `tests/r/jit/loop-vectors.R` (owned copies, growth, `x <- x`,
  values read at entry, deopts and overflow with owned vectors, range
  variables, `max`/`min` with NA, NaN, signed zeros, logicals and a local
  `max`, numeric conditions with NA, leaving at `cat`, `return`, `paste0`,
  `stop`, `warning`, and the shootout program), byte-identical in all tiers.
- Found after filing (own review): native loops do not run the VM's
  `Visible` ops, so `invisible(1); for (...) if (...) return(s)` returned `s`
  invisibly once the `return` was a leave op. Native code now hands the VM
  the visibility at a leave: a forward analysis gives it per op, and only
  where a `return` can read it before an op sets it; a value that differs
  between paths there is tracked at run time. Tracking it everywhere cost
  `scalarFor` +1.0% and `nbody` +1.1%; the analysis made it free.
- Sol review (gpt-6.1-sol, round 1) found that the vector registers restored
  at a leave were only those the leave op reads: in
  `return(sum(x, sum(1L, 2L)))` the VM, after the inner call, read a stale
  register for `x` (the result was 3 for 33). Liveness is now the VM's: a
  backward analysis of every op's reads, definite writes (also on a failing
  guard's jump) and jumps from where native code leaves; a register live
  there that native code wrote without a single type rejects the loop.
  Buffers replaced by larger ones are kept until the run ends, so a register
  can never view freed memory. Case added to the test (fails on the first
  version).
- Gaps found (tests without markers): `tests/r/basics/if-condition-na-message.R`
  (a non-logical NA condition must say "argument is not interpretable as
  logical"; robigo says "missing value where TRUE/FALSE needed", in every
  tier: `coerce::as_condition`), `tests/r/jit/negzero-constant.R` (GNU R's
  compiler pools 0 and -0 as one constant in a compiled function).
- Next: `fannkuchredux` itself needs `:` in native code, indexing with ranges
  and vectors, range assignment, and inlining `rev.default` with a vector
  argument.
