## 2026-11-17: perf-rt guard rows for `[`, `[<-` and the rows no bar cited

- Why: S1's vecsubset work found a regression in `[` (scalar indexing of a
  vector in a loop, +8.4%) that the perf-rt gate did not see. The gate
  measures only the rows its bars cite: calls, S3, fib, binarytrees, and the
  E7 string loops. None of them indexes a vector with `[`; binarytrees
  reaches `[[` only. The foreman asked for a guard: instructions, the 0.5%
  rule, no GNU R bar. Then, for item 1 of the uncovered paths, the rows that
  already existed and no bar cited.
- What:
  - **Three new rows** in `benchmarks/rt/calls-rows.toml`, group `guard`,
    each one path at n = 4000 (the calls rows' n):
    - `guard-subset-dbl`: `s <- s + x[i]` over a double vector of 8,000;
    - `guard-subassign-dbl`: `x[i] <- v` into a double vector of 8,000 (g's
      copy of the global `x` is made once, so the N against 2N difference is
      the in-place `[<-`);
    - `guard-subset-matrix`: `s <- s + m[i, j]` over an 8,000 x 2 double
      matrix.
  - **Eight existing rows** as guards, unchanged: `grow-dbl` (D12's
    appending), `scalars` (D8's `l[[k]] <- v`), and D0's pilot rows
    `d0-attr`, `d0-attributes`, `d0-names`, `d0-unlist`, `d0-paste`,
    `d0-paste0`. `d0-cbind` and `d0-rbind` are not guards yet: robigo-rt
    stops at `.Internal(cbind())`, which is not implemented, so the gate
    would fail. They join when cbind does.
  - **`[[guard]]` in `benchmarks/rt/m1-bars.toml`:** a row with no
    reference, which `perf.py rt-bars` measures with the bars and holds only
    to the 0.5% rule against main's binary. The gate refuses a bars file
    whose guard is not a row, is named twice, or is also a bar's row. The
    report lists a guard as "guard", with no ratio or bar.
- Measured twice under callgrind (robigo-rt built from main 85bf480e,
  `idiom-vs-r.py --rows benchmarks/rt/calls-rows.toml --filter ...`),
  instructions per iteration. The loop with an empty body takes 258 and with
  `v <- 1L` 721, so "path" below is the share of the row beyond a loop that
  assigns a constant, a lower bound; "trips at" is the regression in the
  path that moves its row by the gate's 0.5%:

  | row | run 1 | run 2 | spread | path | trips at |
  |---|---:|---:|---:|---:|---:|
  | `guard-subset-dbl` | 2,123.75 | 2,124.64 | 0.042% | 66% | 0.76% |
  | `guard-subassign-dbl` | 5,044.63 | 5,044.44 | 0.004% | 86% | 0.58% |
  | `guard-subset-matrix` | 3,150.23 | 3,151.08 | 0.027% | 77% | 0.65% |
  | `grow-dbl` | 6,188.75 | 6,188.69 | 0.001% | 88% | 0.57% |
  | `scalars` | 6,647.16 | 6,647.28 | 0.002% | 89% | 0.56% |
  | `d0-attr` | 2,303.07 | 2,302.92 | 0.007% | 69% | 0.73% |
  | `d0-attributes` | 2,554.80 | 2,553.98 | 0.032% | 72% | 0.70% |
  | `d0-names` | 5,431.00 | 5,430.16 | 0.015% | 87% | 0.58% |
  | `d0-unlist` | 11,332.13 | 11,332.81 | 0.006% | 94% | 0.53% |
  | `d0-paste` | 14,196.96 | 14,196.67 | 0.002% | 95% | 0.53% |
  | `d0-paste0` | 13,132.94 | 13,132.39 | 0.004% | 95% | 0.53% |

  The runs differ by at most 0.042%. A 0.5% regression in a path moves its
  row by at least 0.33%, eight times that; S1's 8.4% in `x[i]` would move
  `guard-subset-dbl` by about 5.5%.
- Tests (`scripts/test-perf-rt-bars.py`): a guard only reports without main;
  +1.01% against main fails, +0.40% does not; the file errors; the committed
  guards are the eleven above, every row of the `guard` group among them.
