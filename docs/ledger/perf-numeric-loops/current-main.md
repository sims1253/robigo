# Numeric loops on current main

Baseline: `2fb18b44c593dbec3efe3d00a412d184a8d23536`, release build,
GNU R 4.5.0, Callgrind 3.24.0. Command:

```sh
PATH="$HOME/.cache/robigo-research/R-4.5.0/bin:$PATH" \
LD_LIBRARY_PATH="$HOME/.cache/robigo-research/R-4.5.0/lib/R/lib" \
scripts/perf.py run --metric instr \
  --filter 'awf/Mandelbrot|mandelbrot_|convolution' --jobs 1
```

| Benchmark | Main instructions | GNU R instructions |
|---|---:|---:|
| rbench/awf/Mandelbrot | 201,173,604 | 463,911,482 |
| rbench/real/convolution | 258,117,353 | 1,362,933,727 |
| rbench/real/convolution_slow | 320,310,926 | 1,647,283,414 |
| rbench/real/convolution_v | 272,354,322 | 707,759,895 |
| rbench/shootout/mandelbrot_ascii | 416,952,493 | 719,028,855 |
| rbench/shootout/mandelbrot_naive_ascii | 734,783,586 | 1,024,455,249 |
| rbench/shootout/mandelbrot_noout | 491,000,365 | 813,274,029 |
| rbench/shootout/mandelbrot_noout_naive | 410,313,531 | 782,423,276 |

All eight outputs match the oracle. The awf fixture prints `verified: FALSE`
in both runtimes because its pinned verification data omits parameter 14.

## Stack audit

- #332: main already has nested native regions, heat accumulated across loop
  entries (`vm::loop_heat`), int-or-double `JType::N` values, and integer index
  handling. `compile::seq_length_form` recognizes `seq(length=...)`; native
  treatment must be assessed separately. There is no `Op::Bitwise`.
- #385: main already records unsupported calls as `JitLoop::leaves`, counts
  native back edges, and resumes in the VM. `JType` has no complex variant;
  scalar complex arithmetic therefore cannot run in native loops. The old
  split-variable representation cannot be brought across unchanged.
- #397: guards currently call `guard_ok` individually on every native entry.
  Calls now allow fresh closure instances with shared code, free variables,
  and captured cells. Caching the old stack's closure pointer would discard
  those semantics. Experiment with grouping only loop-environment guards;
  keep call targets, argument operators, and callee operators checked.
- #403: integer arithmetic and conversions still emit NA tests. `JType::N`
  already guarantees non-NA integer values when its tag is integer; a general
  analysis must distinguish that representation from possibly-NA `JType::I`.

The seven stack regression scripts were regenerated with GNU R 4.5.0,
without copying recorded output or adding ratchet markers prematurely.
