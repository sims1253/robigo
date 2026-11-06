# perf-numeric-loops: rbench spectralnorm after #296

Builds on #296 (helper closures recreated per call stay native in loops),
merged into this branch. Metric as in `docs/ledger/perf-numeric-loops.md`.

## 2026-11-04: dot products, `[[<-` of a 1 x 1 matrix, callee guards memoized

- Why: with #296, spectralnorm's inner loops run natively and the outer
  loop (`double(n)`, `ret[[i + 1]] <- u %*% eval_A_col`) in the VM. Profile
  (perf): `matprod_real` 18% and `all_finite` 3% for 828 dot products of
  207 elements per call (two finiteness scans, then the dgemm-shaped
  loop, an inner loop of one element per term); the 1 x 1 matrix result
  sent `ret[[i + 1]] <-` to the complex assignment (`fast_replace`,
  `complex_assign_pre`, 4%); every native loop entry checked the inlined
  callee's operators with `resolves_to_base` (3%).
- What:
  - `matprod_real`, 1 x K times K x 1: one loop `sum += x[j] * y[j]` from
    0, its additions and products in the C code's operand order (sse
    helpers: a sum that is already NaN keeps its payload against another
    NaN, which CI's fastr corpus caught when LLVM was free to commute). GNU R uses dgemv("N") for finite operands (`0 + Y(j) * X(j)` in
    ascending j) and simple_matprod otherwise (`X(j) * Y(j)`): the same
    sums in the same order; a product with a finite factor has the same
    bits either way round, and when X has a NaN simple_matprod's order is
    the one used. So no scan is needed.
  - `set_element`: `x[[i]] <- y` for a double `x` and a length-one double
    `y` with attributes that is not an object takes `y`'s element
    (subassign.c:SubAssign2), in place.
  - `jit_enter`: an inlined callee's guard symbols are checked once per
    (search epoch, environment, its function bindings' version), in its
    `Inline::guards_key`, as the inline runner does.
- Result, peak (rivals.py, this VM): spectralnorm 1.94 ms with #296 ->
  1.58 ms (GNU R 50, Ř 1.05, FastR 0.535; with the native-loop PRs #332
  and #385 too, 1.13 ms). Instructions against main: spectralnorm -3.9%,
  0 slower.
- Verified: `tests/r/linalg/dot-product-bits.R` (NA and NaN payloads in
  both orders, signed zeros, infinities, attributes) and
  `tests/r/jit/loop-inline-guards.R` (operators rebound in the helper's
  frame and globally), recorded with GNU R 4.5.0, byte-identical in all
  tiers; full conformance (only this VM's LAPACK digits fail, as on main);
  fastr corpus slice (matrices, subassignment, vectors) 2606/2606;
  sol-loop 2 rounds, approved; the whole
  fastr corpus has the same failures as main on this VM (312, environment).
- Open: the outer loop stays in the VM (each iteration allocates with
  `double(n)` and calls `%*%`); FastR compiles it whole.
