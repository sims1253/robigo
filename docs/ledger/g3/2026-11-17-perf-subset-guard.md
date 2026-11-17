## 2026-11-17: perf-rt guard rows for `[` and `[<-` by scalar indices

- Why: S1's vecsubset work found a regression in `[` (scalar indexing of a
  vector in a loop, +8.4%) that the perf-rt gate did not see. The gate
  measures only the rows its bars cite: calls, S3, fib, binarytrees, and the
  E7 string loops. None of them indexes a vector with `[`; binarytrees
  reaches `[[` only. The foreman asked for a guard: instructions, the 0.5%
  rule, no GNU R bar.
- What:
  - **Three rows** in `benchmarks/rt/calls-rows.toml`, group `guard`, each
    one path at n = 4000 (the calls rows' n):
    - `guard-subset-dbl`: `s <- s + x[i]` over a double vector of 8,000;
    - `guard-subassign-dbl`: `x[i] <- v` into a double vector of 8,000 (g's
      copy of the global `x` is made once, so the N against 2N difference is
      the in-place `[<-`);
    - `guard-subset-matrix`: `s <- s + m[i, j]` over an 8,000 x 2 double
      matrix.
  - **`[[guard]]` in `benchmarks/rt/m1-bars.toml`:** a row with no
    reference, which `perf.py rt-bars` measures with the bars and holds only
    to the 0.5% rule against main's binary. The gate refuses a bars file
    whose guard is not a row, is named twice, or is also a bar's row. The
    report lists a guard as "guard", with no ratio or bar.
- Measured twice under callgrind (robigo-rt on main b4be2c22 with A5b's
  work, `idiom-vs-r.py --rows ... --filter '^guard-'`), instructions per
  iteration:

  | row | run 1 | run 2 |
  |---|---:|---:|
  | `guard-subset-dbl` | 2,125.02 | 2,124.31 |
  | `guard-subassign-dbl` | 5,043.99 | 5,044.08 |
  | `guard-subset-matrix` | 3,150.92 | 3,151.07 |

  The runs differ by at most 0.034%, well inside the 0.5% rule. The same
  loop without the indexing (`s <- s + x1`, a scalar) takes 681, so `x[i]`
  is about 1,444 of `guard-subset-dbl`'s 2,125 (68%): an 8.4% regression in
  it moves the row by about 5.7%.
- Tests (`scripts/test-perf-rt-bars.py`): a guard only reports without main;
  +1.01% against main fails, +0.40% does not; the file errors; the committed
  guards are exactly the rows of the `guard` group.
