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

## 2026-11-04: memoized guards on native-loop entry

- Why: with the generator inlined (loop cells) every line of a fasta program
  enters a native loop, and entering cost ~4k instructions: per entry 21
  `guard_ok_slow` calls for the loop's own guards and 8 unmemoized
  `resolves_to_base` for the callees' guards, plus a scan of the ops for
  each inlined call's function name.
- What: `guard_ok_slow`, accepting a new frame directly below the memoized
  search-path environment, now records that frame in the memo, so the
  frame's next checks take `guard_ok`'s fast path (before, every check of
  the frame went the slow way again). The callees' guards go through
  `guard_ok` with a memo per guard (`jit::LoopCall`), which also keeps the
  function name each call site names.
- Result: fastaredux, the body repeated 20 times: 2,158.5M -> 2,114.7M
  instructions (-2.4% of the steady state). Gate against the loop-cells
  branch: 3 faster (`fastaredux` -0.69%, `convolution_slow` -0.59%,
  `fasta_2` -0.55%), 0 slower, worst +0.07%. (Wall time on this VM drifted
  by 30% while measuring, so no peak numbers.)
- Verified: `tests/r/jit/loop-callee-guards.R` (operators rebound globally
  between runs, which GNU R's compiled code ignores as robigo does; locally
  in the loop's frame between two loops; in some frames of a function and
  not others; in a callee's environment), recorded with GNU R 4.5.0,
  identical in all tiers; the guard and rebinding scripts of `tests/r/jit`.

## 2026-11-04: character elements and int/double variables in native loops

- Why: after loop cells the naive programs' loops still ran in the VM.
  `fasta_naive` and `fastaredux_naive` copy strings
  (`seq[[i]] <- genelist[[2, ind]]`, `s2[[i]] <- chars[[i]]`); native loops
  knew only double and integer vectors. `fasta_naive_2`'s binary search
  starts `lo <- 1L; hi <- n` and assigns `mid <- (lo + hi - 1L) %/% 2`, a
  double, so `lo` and `hi` change type inside the loop, which rejected it
  (`reason 3`, the only benchmark that hits it).
- What (`jit.rs`, `vm.rs`):
  - `JType::VecS`/`MatS` (character vectors without attributes, matrices
    with only `dim`) and `JType::S`, the address of an element. Reads give
    the address; a write clones the string into the element (a helper, so
    `x[[i]] <- x[[j]]` is right when `i == j`). Where an element is assigned,
    no other string register may be live, so none can stand for a value the
    assignment replaced. A string register live where native code leaves is
    rebuilt as a length-1 character vector.
  - `JType::N`, a number: an f64 value and an integer tag, for a variable
    assigned integers and doubles in a loop. While the tag says integer the
    value is never NA and arithmetic is R's integer arithmetic (native code
    leaves on overflow, NA operands, `%%`/`%/%` by zero); with a double
    operand it is double arithmetic; `/` and `^` give doubles. Variables
    and registers are written back as an integer or a double by their tag.
    Only in loops, never for `for` variables, never as call arguments.
- Rejected on the way: shipping strings alone. The gate showed
  `fasta_naive_2` +1.03%, because the two short `repeat_fasta` copy loops
  (287 and 60 iterations, once per run) now compiled, at ~2.7M instructions
  each, and its main loop still could not. With numbers its main loop
  compiles too, and the benchmark drops by 41%.
- Result, `rivals.py` peak against the loop-cells branch (ms): fasta_naive
  17.8 -> **5.2** (FastR 5.3), fasta_naive_2 21.8 -> **8.9** (FastR 7.3),
  fastaredux_naive 21.0 -> **6.9** (FastR 4.7); the others unchanged. Shot
  improves with peak (fasta_naive 64 -> 41 ms). Gate against the loop-cells
  branch: 3 faster (`fasta_naive` -41.5%, `fasta_naive_2` -41.5%,
  `fastaredux_naive` -41.6%), 0 slower, worst +0.15% (`emptyFor`).
- Verified: `tests/r/jit/loop-strings.R` (vectors and matrices, `[[`/`[`,
  NA strings, in-place reversal, copy-on-modify, encodings, growth, out of
  range and NA indices, names, other types meeting strings, swaps through
  variables, comparisons, two vectors written from each other) and
  `tests/r/jit/loop-numbers.R` (the binary search, types after the loop,
  overflow, `%/%`/`%%` by zero, NA integers and doubles, indices,
  comparisons, stores into double and integer vectors, logicals, negation,
  the `for` variable, `/` and `^`), recorded with GNU R 4.5.0, identical in
  all tiers. Conformance: the same failures as main on this VM.
- Gap left: a register assigned an integer on one branch of `if` and a
  double on the other (`x <- if (c) 1L else 0.5`) has no single type at the
  join, so such loops stay in the VM; widening registers at joins would need
  conversions on the incoming edges.

## 2026-11-04: `for (j in seq(from, to, by))` without calling `seq`

- Why: pidigits' bigint multiplication loops over
  `seq(y_start, 1L, -1L)` and `seq(x_start - 1L, 1L, -1L)` with a few limbs
  each. GNU R's `Rprof` puts 45% of pidigits in `seq`/`seq.default`; in
  robigo one such loop cost 8.5 us (GNU R 5.5 us), against 0.02 us for
  `(x_start - 1L):1L`. robigo was 2x behind GNU R at peak.
- What (`compile.rs`, `vm.rs`): `Op::ForSeqBy`, like `ForSeqLength`. The
  arguments are leaves, `leaf +/- leaf` or `-leaf`, guarded with `seq`
  and the operators. When they are integers (not NA, computed by
  `arith_fast`, so no warning or dispatch can happen) and the call reaches
  base's `seq.default` (no `seq.integer`/`seq.numeric` method), the loop gets
  the range seq.default's `by` branch computes: `from` alone when
  `from == to`, else `from + (0:n) * by` with `n = as.integer((to -
  from)/by)`. Every other case (doubles, logicals, objects, a zero or
  wrong-signed `by`, NA, overflow) evaluates the call, so the errors,
  warnings and calls are GNU R's.
- The method check costs three `lookup_method`s (~0.8 us, 10% of pidigits):
  for function frames below the global environment, the part that depends
  on the global environment and base is memoized by the search epoch (their
  function bindings bump it); the frames and base's methods table are
  checked each time.
- Rejected: the arm inline in `run_with`. It cost `nbody_naive` +0.70% and
  `superWhile` +0.57%, by codegen of the dispatch loop alone; with both
  helpers `#[inline(never)]` they are +0.02% and +0.11%.
- Result: pidigits peak 21.4 -> **7.1 ms** (GNU R 10.3, FastR 0.94).
  Gate (full suite against main bf7332ee): 1 faster (`pidigits` -26.2%),
  0 slower, worst +0.28% (`regexdna`).
- Verified: `tests/r/basics/for-seq-by.R` (ranges, `from == to`, extreme
  integers, doubles and logicals, every error of the `by` branch and its
  call, arithmetic arguments, an overflowing argument warning once, methods
  in the global environment, a frame, base's table and locally, rebound
  `seq`, `-` and `+`, promises forced once and in order, objects),
  recorded with GNU R 4.5.0, identical in all tiers.

## 2026-11-04: `x[i]` and `x[i] <- v` with index vectors in the VM

- Why: fasta_2 and fasta_3 search vectorised (`mid[inds] <- (lo[inds] +
  hi[inds] - 1L) %/% 2`, `hi[ge] <- mid[ge]`, `psum[mid]`), ~45k such ops
  per run on vectors of 60. `Op::Index` and `Op::SetIndex` only had scalar
  and range fast paths, so every one went to `index_slow`, which evaluated
  the whole call again in the AST interpreter (92k calls at ~5,000
  instructions, 19% of fasta_3), and `set_index_slow` (14%).
- What (`vm.rs`, in the out-of-line slow functions, so `run_with` is
  unchanged): an index vector that is integer or double, without
  attributes, every element in `1..=length(x)`, on an attribute-free
  double, integer, logical or character `x`. `x[i]` gathers the elements;
  `x[i] <- v` writes them in place into this frame's (unshared) `x`, `v`
  attribute-free with a length dividing the index's (recycled, as R does
  silently then), an integer `x` becoming double for a double `v`
  (SubassignTypeFix). Everything else (NA, zero, negative, out of range,
  names, dims, lists, other coercions) takes the general path as before.
- Rejected on the way: building the positions with
  `collect::<Option<Vec<_>>>()`: 1,900 instructions per call of 60 (it
  grows the vector); validating in place and indexing directly costs a
  fraction.
- Result, local min of 40 runs (ms): fasta_2 18 -> 14 (FastR 14.9),
  fasta_3 23 -> 18 (FastR 12). Gate (full suite against main bf7332ee):
  7 faster, 0 slower: `fannkuchredux` -18.4%, `fasta_2` -8.6%, `fasta_3`
  -8.4%, `sortorder` -2.9%, `fastaredux` -1.1%, `dataframe` -0.9%,
  `vecops` -0.6%; worst +0.05%.
- Verified: `tests/r/vectors/index-vectors-vm.R` (every type of `x`, integer
  and fractional double indices, duplicates, empty, NA/NaN, zero, negative,
  out of range and growth, names, matrices, factors, lists, logical, named
  and factor indices, coercions both ways, recycling with and without the
  warning, zero-length values, copy-on-modify, `a[a] <- a * 10`, a global
  copied into the frame, a lazy index, the fasta_3 search), recorded with
  GNU R 4.5.0, identical in all tiers.

## 2026-11-04: cat() and paste(collapse=) of strings without a copy per element

- Why: every fasta program writes each 60-character line with
  `cat(x, "\n", sep = "")` or `cat(paste(x, collapse = ""), "\n", sep = "")`.
  Per call of 60 one-character strings `cat` took ~16.6k instructions and
  `paste` ~17.8k (~280 per element): `paste` built a `String` per element
  and joined them, `cat` allocated one per element (`encode_element_cat`),
  built `PrintParams` (five option lookups) on every call, and took a
  modulo for the separator per element. With the naive programs' loops
  native, these were half of what was left of fastaredux_naive.
- What (`builtins/paste.rs`, `builtins/io.rs`): `paste` with `collapse`
  writes every piece straight into one buffer of the right size; `cat`
  borrows string elements, builds `PrintParams` only for numbers, reserves
  the output, and writes a character vector element by element when not
  filling (widths matter only for `fill`). The bytes are the same.
- Result (us per call, release): `cat(x, "\n", sep = "")` 2.45 -> 1.60,
  `paste(x, collapse = "")` 1.80 -> 1.00, both together 3.60 -> 2.15 (GNU R
  18.3, 3.75, 6.50). Gate (full suite against main bf7332ee): 7 faster (the
  whole fasta family, -1.3% to -2.9%), 0 slower, worst +0.02%.
- Verified: `tests/r/io/cat-paste-strings.R` (NA and empty strings, vector
  separators, numbers among strings, empty and NULL arguments, `fill`,
  `labels`, `paste` with several arguments, recycling, `collapse = NULL`,
  empty inputs, lists, encodings), recorded with GNU R 4.5.0, identical in
  all tiers and to main; the FastR corpus slices for cat/paste/format/print/
  sprintf/strsplit/writeLines/printing/connections: 591/592, 0 regressions.

## 2026-11-04: integer `%%` in native loops without a helper call

- Why: the Cranelift IR of fastaredux_naive's loop (dumped locally) showed
  every integer `%%` (the generator's `%% 139968L`) as a call of `h_imod`,
  and every integer index going through floating point.
- What (`jit.rs`): `x %% y` on integers is `urem` when `x >= 0` and
  `y > 0` (neither is NA then, the result is `h_imod`'s), the helper
  otherwise, in a cold block. The integer index part was dropped from this
  change: #331 (vector loops) makes the same one.
- Rejected: shipping the index part here too (gate: `convolution` -13.9%,
  `robigo/nbody` -13.5%, `vectorFor` -13.4%): a duplicate of #331.
- Result: fastaredux on top of loop cells, body repeated 20 times:
  2,158.5M -> 2,147.2M instructions (-0.6% of the steady state). Gate
  against main: 0 faster, 0 slower (on main no fasta loop with `%%` is
  native yet).
- Verified: `tests/r/jit/loop-int-index-mod.R` (`%%` over every sign
  combination, zero and NA, the fasta generator, and integer indices),
  recorded with GNU R 4.5.0, identical in all tiers.

## 2026-11-04: `%/%` of integer-valued doubles without long double

- Why: fasta_naive_2's search computes `mid <- (lo + hi - 1L) %/% 2` (a
  double divisor), and fasta_3's does it on vectors. Every element went
  through `myfloor`'s long double correction, which robigo computes in
  software (`ldouble.rs`): `myfloor` was 4% of fasta_3.
- What (`robigo-numerics/src/arith.rs`): after `myfloor`'s early returns
  (|q| >= 1 from there), integer-valued operands within 2^53 return
  `floor(x1 / x2)`. That is the long double result: rounding `x1 / x2` up to
  an integer would take |x1| > 2^53, so `floor(q)` is the exact quotient's
  floor, the long double remainder is exact with |tmp| < |x2| and the sign
  of `x2`, and its quotient's floor (0 or -0) adds nothing to the nonzero
  `floor(q)`.
- Result: on top of the other changes of this track, fasta_naive_2 (body
  repeated 20 times) 1,174.4M -> 1,112.0M instructions (-7.6% of the steady
  state). Gate against main: 2 faster (`fasta_3` -0.77%, `fasta_naive_2`
  -0.63%), 0 slower.
- Verified: a unit test compares the fast path with the long double formula
  bit for bit on 100k+ integer pairs of every magnitude to 2^53 and both
  signs; `tests/r/math/idiv-doubles.R` (signs, zero and -0, 2^53 and
  around, infinities, NaN/NA, fractions, huge quotients, loops, vectors),
  recorded with GNU R 4.5.0, identical in all tiers and to main.
