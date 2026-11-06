# perf-nbody/slices

Part of the perf-nbody track: the metric, the starting numbers and the
profiles are in `docs/ledger/perf-nbody.md`.

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
- Review (sol-loop, gpt-6.1-sol, 1 round): approved with two nits.
  Fixed: the fast path read extents whose `dim` vector has names (it now
  declines them). That showed a gap in robigo's `[` itself, which drops
  those names in every tier: `tests/r/subset/dim-names-slice.R`, no ratchet
  marker. Declined: an RHS that rebinds `[<-` is not seen because the
  setter's guard runs before the RHS. `main`'s `Op::SetIndex2` does the same
  (`m[1, 1] <- { \`[<-\` <- function(x, ..., value) x; 1 }` writes 1, GNU R
  leaves 0); a gap of the VM's setter guards, not fixable as a `tests/r`
  script since the tiers differ.
- Rebased onto main 7a0d27bd, which made stores in effect position drop
  their value (#357, `take`): `Op::SetIndexN`/`Op::SuperSetIndexN` gained
  `take` like `SetIndex2`. Review of the rebase (sol-loop, gpt-6.1-sol)
  found the fast paths still left the value in its register after copying
  its elements, so an environment in one of its attributes outlived the
  next `gc()`; they now clear it.
  `tests/r/env/vm-dead-registers-slices.R` (fails in the VM tiers without
  that).
- Against main 7a0d27bd (its dispatch loop keeps only hot paths inline):
  the three ops' fast paths are out of line too (`index_n_fast`,
  `set_index_n_fast`, `super_set_index_n_fast`), as main's store ops are:
  `profiler-microbenchmark` +0.14% -> +0.01% (code size in `run_with`
  alone), `nbody_2`/`_3`/`_naive_2` unchanged.

