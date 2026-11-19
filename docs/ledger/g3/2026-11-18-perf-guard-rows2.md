## 2026-11-18: perf-rt guard rows for paths no row reached

- Why: #826 listed the common VM paths that no perf-rt row exercised, and
  the foreman asked for rows for them (item 2), plus a unit test that
  `perf.py rt-bars` wires guards through.
- What: six rows in `benchmarks/rt/calls-rows.toml`, group `guard`, at
  n = 4000, all listed under `[[guard]]` in `benchmarks/rt/m1-bars.toml`
  (instructions, the 0.5% rule against main, no GNU R bar):
  - `guard-vec-arith`: `y <- x * 2 + x` on a 1,000-element double vector;
  - `guard-vec-sum`: `s <- sum(x)` on it;
  - `guard-dots`: `v <- f(i, 1)` with `f <- function(...) h(...)`;
  - `guard-dollar-assign`: `l$a <- i` on a list of two, in place;
  - `guard-while`: `k <- 0L; while (k < 10L) k <- k + 1L`;
  - `guard-repeat`: the same ten steps in `repeat`, left by `break`.
- Measured twice under callgrind on main e20eef0d1's robigo-rt,
  instructions per iteration. The loop with an empty body takes 246 and
  with `v <- 1L` 714. "path" is the share of the row beyond the latter, a
  lower bound. "trips at" is the regression in the path that moves the row
  by the gate's 0.5%:

  | row | run 1 | run 2 | spread | path | trips at |
  |---|---:|---:|---:|---:|---:|
  | `guard-vec-arith` | 88,471.4 | 88,471.3 | 0.000% | 99% | 0.50% |
  | `guard-vec-sum` | 20,198.4 | 20,198.2 | 0.001% | 96% | 0.52% |
  | `guard-dots` | 8,653.6 | 8,653.6 | 0.000% | 92% | 0.55% |
  | `guard-dollar-assign` | 4,489.2 | 4,490.4 | 0.026% | 84% | 0.60% |
  | `guard-while` | 8,404.6 | 8,404.8 | 0.001% | 91% | 0.55% |
  | `guard-repeat` | 8,536.0 | 8,535.1 | 0.011% | 92% | 0.55% |

- Test (`scripts/test-perf-rt-bars.py`): `cmd_rt_bars`, with the
  measurements mocked, measures a guard with the bars on robigo-rt and on
  main, shows it as a guard, and fails when it is 1% slower than main.
  Without main it measures only robigo-rt and passes.
