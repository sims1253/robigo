# perf-simple-loops: loops that only count and copy

Metric: `scripts/rivals.py` peak (fastest warmed-up iteration) and shot on
`rbench/simple/{scalarFor,emptyFor,emptyWhile,scalarWhile,bytecodes,vectorFor,superWhile}`
against GNU R 4.5.0, Ř and FastR; gate: conformance (all tiers),
`scripts/perf.py compare` (no benchmark more than 0.5% slower).

## 2026-11-04: round 1, loops in closed form

- Base: origin/main bf7332ee. Rivals on this VM (peak, seconds): Ř 6.9e-8
  `emptyFor`, 7.2e-8 `scalarFor` (it deletes the loops), 1.22e-3
  `emptyWhile`, 6.6e-4 `scalarWhile`, 3.5e-5 `bytecodes`, 2.09e-3
  `vectorFor`; FastR and GNU R slower on all. robigo: 1.7e-3, 4.8e-4,
  1.75e-3, 9.5e-4, 1.5e-4, 2.6e-3.
- Where the time went: the native loops were already at 1-3 cycles per
  iteration (emptyWhile's loop is latency-bound on `fadd` plus a compare
  chain; Zen 5). `bytecodes` (25 copies of `i` per iteration) spilled.
  Ř's numbers mean it removes `for` loops and keeps `while` loops at the
  `fadd` latency. Only removing the loops beats that.
- Found on the way: in the rivals harness, after 512 calls the closure tier
  (`jit/closure.rs`) compiles `execute` whole, and its loop is ~3x slower
  than the loop tier's (5.4 ms vs 1.75 ms for emptyWhile): regalloc2 keeps
  the loop-invariant `n` in a stack slot and stores and reloads it on every
  iteration (it is live across the frame-init call at entry). On main the
  peak therefore came from early batches; the steady state was 5.4 ms.
  Not fixed here (see below).
- Change: `jit/summary.rs`. A loop whose iterations only count and copy is
  replaced by its final state, when it certainly runs to its end in native
  code. One iteration is executed symbolically; every variable it assigns
  must be an induction variable (`x <- x + c`, `x <- x - c`, integer `c`)
  or a function of induction variables, the `for` element and variables
  the loop does not assign, made of operations that never leave native
  code (double `+ - * / ^`, comparisons). The trip count is the `for`
  sequence's remaining length, or for `while (iv OP bound)` the first `k`
  at which the comparison fails, computed in exact integer arithmetic.
  At run time the summary declines (and the native loop runs) unless every
  induction variable is an integer-valued double within 2^53 all the way
  (or an integer without overflow), the bound is not NA, the loop ends,
  and a `for` sequence is not empty. A value at the last iteration that
  would be its start (`k = 0`) keeps the start's bits (`-0`).
  - Loop tier: tried in `vm::jit_enter` on the same value buffer as the
    native code, which it leaves as the native code would at its exit.
  - Closure tier: at a `LoopEnter` with a closed form, the loop's scalars
    go to a stack buffer, a helper runs the summary, and on success they
    are reloaded and the code jumps to the `LoopExit`.
  - Closure tier: locals that no natively executed op reads are not
    assigned at all (`Inl::dead`): nothing can see the frame of a closure
    run natively, and a run that leaves is replayed by the VM. This is what
    lets `scalarFor`'s `x <- 0; for (i in 1:n) x <- i` (a double, then an
    integer) compile whole.
  - `Summary::run` allocates nothing (first version: 780 instructions per
    call in Vecs and a boxed closure; now ~120).
  - `ROBIGO_JIT_SUMMARY=0` turns summaries off; `ROBIGO_JIT_DUMP=1` prints
    the Cranelift IR and machine code of every native loop and closure.
- Result (peak, seconds, `rivals.py`): emptyWhile 1.75e-3 -> 1.7e-7
  (Ř 1.22e-3), scalarWhile 9.5e-4 -> 1.8e-7 (Ř 6.6e-4), bytecodes 1.5e-4
  -> 2.9e-7 (Ř 3.5e-5), scalarFor 4.8e-4 -> 9.8e-8 (Ř 7.2e-8), emptyFor
  1.7e-3 -> 1.2e-7 (Ř 6.9e-8). What is left on the `for` pair is the call
  path, ~2200 instructions per `.body()` of the harness: its top-level
  loop's flush of `.j` to the global environment, `native_site_call`,
  the closure tier's frame init/release of 10 `RVal` slots for a `NULL`
  result, `closure::run`.
- Verified: `tests/r/jit/loop-closed-form.R` (while and for loops, both
  tiers, NA bounds, non-integer starts, `-0`, 2^53, empty sequences,
  integer overflow with its warning, locals that change type, a `stop()`
  in a loop), recorded with GNU R 4.5.0; differential runs of the same
  cases with summaries on and off.
- Open: the closure tier's spilled loop invariants (regalloc2; every
  closure-tier loop pays it); the call path above; `vectorFor` and
  `superWhile` (#294).

## 2026-11-04: round 2, vectorFor (#337)

- Why: vectorFor peak 2.63 ms, Ř 1.97. `perf`: 80% in the native loop
  (~5 cycles per element), 9% memmove, 6.5% memset.
- Changes, each measured:
  - integer index in native code (`Gen::index`): was int -> double ->
    range check -> trunc -> saturating convert -> -1 -> bounds check; now
    `i - 1` and one unsigned compare (NA, 0, negatives are below 1 and
    leave as before). 2.63 -> 1.39 ms.
  - the memmove was the first `x[[1]] <- 1L` coercing logical to integer
    by copying. Making the coercion reuse an unshared logical vector
    (`Lgl` is `i32`) did nothing at first: the vector was shared with VM
    register 0, which kept the value of `x <- vector(...)`, and with the
    VM's clean slot cache. `SetVar { moves }` moves a value whose
    assignment is a statement (`Want::Effect`); `set_index_slow` empties
    the (just flushed) cache before the complex assignment. 1.39 -> 1.15 ms.
  - left: the memset of `vector(length = n)` (13%) and a 2-cycle store
    loop.
- Result: vectorFor peak 1.15 ms (Ř 1.97); `perf.py compare`: 11 faster
  (convolution -13.9%, nbody -13.5%, matrixFor -5.8%), 0 slower, worst
  +0.21%. rbench `convolution` peak 2.56 -> 1.32 ms.
- vectorFor shot: 0.043 s here (GNU R 0.082); the scoreboard's 0.2 s did
  not reproduce.
- Not done: `ROBIGO_CONF_UPDATE=1` over the whole suite also adds markers
  for scripts that pass on this VM but are not ratcheted; only this
  round's markers were kept.
