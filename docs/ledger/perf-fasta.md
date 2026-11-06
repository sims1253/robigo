# perf-fasta: the fasta/fastaredux family and pidigits

Metric: `scripts/rivals.py` peak (fastest warmed-up iteration) against GNU R,
Ř and FastR; gate: `scripts/perf.py compare` of instruction counts against
main (no benchmark more than 0.5% slower) and conformance in all tiers.

Baseline (main 5ce8d760, `~/scoreboard.txt`), peak seconds:

| benchmark | robigo | GNU R | Ř | FastR |
|---|---|---|---|---|
| fasta | 0.013 | 0.0535 | 0.0423 | 0.00481 |
| fasta_2 | 0.0253 | 0.0855 | 0.0795 | 0.0149 |
| fasta_3 | 0.0245 | 0.0495 | 0.0408 | 0.012 |
| fasta_naive | 0.0168 | 0.037 | 0.0191 | 0.00528 |
| fasta_naive_2 | 0.019 | 0.0262 | 0.0238 | 0.00731 |
| fastaredux | 0.0158 | 0.0575 | 0.0315 | 0.012 |
| fastaredux_naive | 0.0182 | 0.0423 | 0.0255 | 0.00469 |
| pidigits | 0.0144 | 0.0103 | 13.5 | 0.000937 |

## 2026-11-04: where the time goes

- Profiled `fastaredux_naive` (callgrind and perf, the body repeated so that
  warmup does not count): 38% in `vm::run_with`, 27% under
  `native_site_call`. The inner `for (i in 1:line)` loop of every program
  ran in the VM. `ROBIGO_JIT_TRACE=1` says why:
  - every program's loop calls the generator `myrandom(1)` /
    `random_next_lookup()`, which assigns a global with `<<-`; the loop JIT
    inlined only pure closures without free variables, so the loop stayed in
    the VM and paid ~100 ns per call through `native_site_call`;
  - the naive programs also copy strings (`seq[[i]] <- genelist[[2, ind]]`),
    and native loops know only double and integer vectors.
- Segments of `fasta` (ms per run, robigo): generator loop 5.9, `outer` 6.8,
  `colSums` 3.2, matrix index 0.9, `cat` 2.0. FastR does the whole run in
  4.8 ms, so every piece has to get faster, not just one.

## 2026-11-04: loop cells — native loops calling closures that use `<<-`

- What (`jit.rs`, `vm.rs` `inline_call`/`loop_callee`/`jit_enter`): a
  closure called inline from a native loop may read scalar variables of its
  enclosing environments and assign them with `<<-`. Each such variable is a
  *cell* of the loop: read from its binding when native code is entered,
  written back (when assigned) when it leaves. An inlined call assigns a
  copy, which becomes the cell's value only when the call completes, so a
  call that leaves native code midway (an integer overflow, say) leaves the
  cell as it was and the VM replays the call from scratch.
- Why it is safe: no R code runs in a native loop, so only the loop can see
  the binding, and the loop is rejected if it names the variable itself.
  At entry the binding must be plain (written cells) and the same for every
  call site; a closure defined in the loop's frame first gets the frame's
  pending assignments flushed (as `native_site_call` does). After writing
  back, the frame's cached copies of the variable are dropped.
- Result, `rivals.py` peak against main bf7332ee (same VM, budget 4 s):
  fasta 14.9 -> 10.9 ms, fasta_2 21.9 -> 17.3, fastaredux 13.5 -> 7.6,
  fasta_3 25.0 -> 24.3; the naive programs unchanged (their loops also copy
  strings). Gate (`perf.py compare`, full suite against main): 5 faster,
  0 slower, 0 broken: `fastaredux` -26.8%, `fasta` -19.4%, `fasta_2`
  -16.4%, `fasta_3` -13.7%, `profiler-microbenchmark` -69.2%; worst +0.13%
  (`fannkuchredux_naive`).
- Verified: `tests/r/jit/loop-cells.R` (global and frame-local generators,
  pending assignments, shared cells, overflow midway, NA conditions, type
  changes, a formal named like the variable, locked and active bindings, the
  loop naming the variable, unbound targets, vectors, grandparent frames,
  `break`, shadowing), recorded with GNU R 4.5.0, identical in all tiers.
- Gap found: `tests/r/jit/active-binding-compiled-reads.R`. GNU R's bytecode
  runs an active binding's function twice per read once the reading closure
  is compiled; robigo runs it once in every tier.
