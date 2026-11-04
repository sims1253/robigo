# perf-numeric-loops: complex scalars and leaving ops in native loops

Stacked on the native-loops round (#332). Metric and gate as in
`docs/ledger/perf-numeric-loops.md`.

## 2026-11-05: mandelbrot_naive_ascii 47 -> 3.0 ms (FastR 4.47, Ř 16.9)

- Why: the benchmark's pixel loop computes with complex scalars
  (`z <- z * z + c`, `abs(z)`), which native loops did not have, so all of
  it ran in the VM (`leaf_op_slow`/`complex_op`, 40 ms at peak); and every
  eighth pixel calls `cat()`, which rejected the loop.
- What (`jit.rs`, `vm.rs`, `coerce.rs`):
  - `JType::C`: a complex scalar as an f64x2 (two entries of the value
    buffer, unaligned loads and stores). `+ - *` with complex, double and
    integer operands (an integer NA leaves), negation, `abs()` by the same
    `hypot` the VM calls, complex constants. A result with a NaN part
    leaves: which payload survives and `__muldc3`'s recovery of
    infinities are the VM's to compute; without NaN, the IEEE results do
    not depend on operand order.
  - Ops native code does not have (calls, builtins, `Eval`, specials)
    leave to the VM where they are reached, if some path from the loop's
    head comes back to it without them (otherwise the loop is not
    compiled). After leaving at the loop's own level following a whole
    iteration (a count of the head), the VM enters native code again at
    the next back edge. The count is kept only in loops that can leave
    (in all loops it cost `emptyFor` +3.4%).
  - `as.integer`, `as.double`, `as.numeric` of one scalar natively
    (`IntegerFromReal`, leaving where it warns); `if`/`while` conditions on
    integers and doubles.
  - A register's place in the value buffer has its widest type's size
    (registers are reused with other types: a complex value overwrote the
    next register's place, found by `tests/r/jit/loop-complex.R`).
  - `if (NA_integer_)` and `if (NaN)` said "missing value where TRUE/FALSE
    needed" in every tier; GNU R says "argument is not interpretable as
    logical" for a non-logical NA (eval.c:asLogicalNoNA).
- Rejected, measured: a head count in every native loop (`emptyFor` +3.4%,
  `emptyWhile` +1.3%); resuming after leaving inside a nested loop
  (needs the outer loop's record; only the loop's own level resumes).
- Result: peak `mandelbrot_naive_ascii` 47 -> 3.0 ms (this VM, loaded).
  Instructions against main 1e285547: 14 faster, 0 slower;
  `mandelbrot_naive_ascii` -66%.
- Verified: `tests/r/jit/loop-complex.R`, `loop-leave.R` recorded with GNU
  R 4.5.0, byte-identical in all tiers; full conformance (the only
  failures are this VM's LAPACK digits in `linalg/battery`, `linalg/svd`,
  as on main); fastr corpus slice (complex, loops, arithmetic, if) 957/958
  with no regressions; sol-loop 1 round, approved.
- Open: `abs()` is glibc's `hypot`, a fifth of what is left.
