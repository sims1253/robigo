# perf-nbody: the nbody family and the spectralnorm variants

Metric: peak wall time in `scripts/rivals.py` against GNU R 4.5.0, Ř and
FastR (fastest warmed-up iteration), without losing the fresh-process
time. Gate: `scripts/perf.py compare` of instruction counts against main
(no benchmark more than 0.5% slower) and `cargo test --release -p
robigo-cli --test conformance` (interpreter, VM and VM+JIT agree).

Starting point (main 5ce8d760, peak seconds; robigo / GNU R / Ř):
`nbody_2` 0.033 / 0.0249 / 0.0165, `nbody_3` 0.048 / 0.0488 / 0.0475,
`nbody_naive` 0.0229 / 0.0423 / 0.00494, `nbody_naive_2` 0.0565 / 0.054 /
0.0335, `spectralnorm_alt` 0.044 / 0.039 / 0.0345, `spectralnorm_alt_3`
0.0166 / 0.0495 / 0.00295, `spectralnorm_naive` 0.035 / 0.0535 / 0.00381.

Profiles (perf, 20-40 repetitions of the benchmark body in one process):

- `spectralnorm_naive`: 82% `inline_site_call` + `run_with`. The inner loop
  `ret[[i]] <- ret[[i]] + u[[j + 1]] * eval_A(i - 1, j)` never ran
  natively: the loop JIT accepted only constants and variables as
  arguments of an inlined call (`jit: unsupported at 30: Call eval_A`).
- `spectralnorm_alt_3`: the same loop shape, but `eval_A` reads
  `eval_A_cache[[i, j]]`, a free variable, which the loop JIT's inline
  calls do not have (`InlineCall` is "without free variables").
- `nbody_naive`: 66% `run_with`: every loop assigns `body_vx[[i]] <<- ...`,
  which the loop JIT rejects (#294 covers scalar `<<-` only).
- `nbody_2`, `nbody_3`, `nbody_naive_2`: `eval_sym`, `find_fun`,
  `complex_assign_pre`, `eval_keep_missing`: `x[, i]` and
  `d[i, j, ] <- v` (empty indices) had no VM op and ran in the AST
  interpreter.
- `spectralnorm_alt`: 76% `matprod_real` (`u %*% M`, n = 1083).

## 2026-11-04: `u %*% M` keeps eight column sums in flight

- Why: `spectralnorm_alt` is 20 products of a vector by a 1083 x 1083
  matrix. GNU R's dgemv("T") sums one column at a time, a chain of
  dependent additions: robigo ran at one addition's latency per element
  (76% of the profile).
- What (`builtins/array.rs` `matprod_real`): eight columns at a time, each
  summed in exactly the reference order, so eight chains are in flight.
  Then the NaN/Inf scan GNU R does to choose `simple_matprod` was 34% (a
  second pass over 9 MB): for a row vector both paths are the same sums in
  the same order, and with `x` finite the same bits whatever `Y` holds (a
  product with a finite factor has the same value and NaN payload either
  way round; the sum never is -0), so only `x` is scanned. Columns that
  come out NaN are summed again in GNU R's operand order: LLVM may swap
  the operands of the unrolled additions, which decides the payload
  where an NA meets a NaN (the new test caught it: `NaN` printed for
  `NA`).
- Result: instructions `spectralnorm_alt` -47.4%, `spectralnorm_alt_2`
  -6.3%, `spectralnorm` -0.8% (gate: 3 faster, 0 slower); 30 runs in one
  process 0.74 s -> 0.29 s; peak 0.0348 -> 0.0114 s (Ř 0.0345, GNU R
  0.039), shot 0.071 -> 0.042 s (Ř 0.137).
- Verified: `tests/r/linalg/matprod-row-vector.R` (37 x 21, eight-wide
  blocks and the remainder; NA, NaN, ±Inf in the matrix and in the vector,
  zeros meeting Inf, signed zeros, overflow from finite values, integer
  and logical operands), recorded with GNU R 4.5.0, byte-identical in all
  three tiers.
## 2026-11-04: array slices by scalar and empty indices in the VM

- Why: `x[, j]`, `d[i, j, ]` and their assignments had no VM op. The VM
  handed the whole call to the AST interpreter (`find_fun` for `[`,
  promises, the indices evaluated again, `complex_assign_pre`). Per op
  (1e6 in a compiled loop): `x[, j]` 252 ns (GNU R 350), `d[i, j, ] <- v`
  347 ns (GNU R 237), `bv[, i] <<- bv[, i] + 1` 653 ns (GNU R 462).
- What (`compile.rs`, `vm.rs`): `Op::IndexN`, `Op::SetIndexN`,
  `Op::SuperSetIndexN` for `[` of a variable with two or more untagged
  indices that are each empty, a variable or a numeric constant
  (`indexn_form`; reading those can be observed by nothing, so nothing is
  evaluated before `[` would). The fast path takes a plain atomic array
  (only `dim`) with in-range scalar indices: the read builds the slice
  with R's `drop = TRUE` dims (subset.c:ArraySubset/DropDims); the
  assignment writes in place a value of the same or a lower type, without
  a class, whose length divides the slice (subassign.c:ArrayAssign), on the
  local binding (`own_var`) or the one `<<-` assigns (as
  `super_set_element`). Anything else runs the call as before.
- Result: per op `x[, j]` 252 -> 108 ns, `d[i, j, ] <- v` 347 -> 53 ns.
  Instructions: `nbody_naive_2` -39.5%, `nbody_3` -28.0%, `nbody_2`
  -24.4%, `mandelbrot_noout_naive` -8.6% (gate against main: 4 faster,
  0 slower).
- Verified: `tests/r/subset/array-slices.R` and
  `tests/r/subset/array-slice-assign.R` (matrices and 3-/4-d arrays,
  every drop shape, 1-row/1-column/1x1x1, integer/logical/double, names,
  extra attributes, characters, complex, lists, data frames, `[.foo` and
  `[<-.foo` methods, vector/logical/NA/zero/negative/out-of-range/missing
  indices, wrong rank, a rebound `[`, `drop = FALSE`, type changes,
  factors and named values, recycling and its errors, copies of shared
  arrays, a global array assigned locally, `<<-` into global and enclosing
  frames with local shadows and aliases, a missing target, a locked
  binding, visibility), recorded with GNU R 4.5.0, byte-identical in all
  three tiers. Conformance: as main. `vectors/giant-seq-gap.R` (a 16 GB
  `1:-2147483648`, not ratcheted) can differ between tiers under memory
  pressure on this VM, with main's binary too.
