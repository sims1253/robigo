# perf-vm: the VM and the JIT

Metric: `scripts/perf.py run --metric instr`, geometric mean of robigo/GNU R
instructions over programs whose output matches GNU R's; the number of
benchmarks faster than GNU R. Gate: `scripts/perf.py compare` (no benchmark
more than 0.5% slower) and `cargo test --release -p robigo-cli --test
conformance` (interpreter, VM and VM+JIT against each other).

## Setup notes (this machine)

The host has GNU R 4.6.1 and glibc 2.44. Both runtimes are measured in a
`debian:trixie` container with `r-base-core=4.5.0-3`, valgrind 3.24 and
python3 (as CI), robigo built in the same container (the host binary needs
glibc 2.44's `sinh`/`cosh`, and would use a different libm). The local
baseline reproduces CI's `perf-main` artifact of `main` 7bb554c within 0.1%
(geomean 0.730 locally, 0.729 on CI).

## 2026-10-28: call sites find functions bound in local frames without the slow path

- why: in `shootout/spectralnorm` (1.82x GNU R) the inner loop calls
  `eval_A`, a pure helper defined in the enclosing function. Call sites
  looked functions up quickly only on the search path:
  `call_site_fun_fast` gave up at the first local frame that might bind
  the name, so the call was never inlined and went through promises, a
  frame and a flush (17.6k instructions per iteration).
- what: a local frame that binds the name to a closure, builtin or forced
  promise of one returns it (as `findFun` would; non-functions are
  skipped); unforced promises, active bindings and missing arguments still
  take the slow path.
- tests: `tests/r/basics/local-fun-calls.R` (helpers, shadowing by a
  non-function, function arguments forced lazily, redefinition in the
  loop, missing and active bindings, local recursion, warnings).
- result: 17.6k -> 3.1k instructions per inner iteration;
  `spectralnorm` -69.1% (1.82 -> 0.56), `profiler-microbenchmark` -3.7%,
  `spectralnorm_alt_3` -1.8%, `spectralnorm_naive` -1.1%, `profiler-rsa`
  -0.6%, none slower; geomean 0.705 -> 0.693, faster on 52 -> 53; kept.

## 2026-10-28: `x[[i, j]]` in the VM; index promises forced in place; cheaper slot write-backs

- why: `simple/matrixFor` (`x[[i, j]] <- 5` in a double loop) cost 5.45x
  GNU R: only `x[i, j]` had VM ops, so `[[` with two indices ran the
  interpreter's complex assignment (~6.4k instructions per element; GNU R
  ~600).
- what:
  1. `Index2`/`SetIndex2` take `[[` as well (`two`; `x[[i, j]] <- v` stores
     list elements as `[[<-` does).
  2. That alone made `spectralnorm_alt_3` 13.6% slower: its
     `eval_A <- function(i, j) eval_A_cache[[i, j]]` reaches `Index2` with
     `i`, `j` unforced promises, so the fast path always failed and paid for
     a fallback. `Index`/`Index2` now force index promises themselves when
     `x` is not an object, in R's order (x, then the indices: eval.c
     `R_DispatchOrEvalSP` + `evalListKeepMissing`), only when
     `R_isMissing` would say "not missing" (promise of a non-symbol, or of
     a symbol bound to a plain value in the promise's frame); `x`'s value is
     held from before the forcing, and a failed fast path calls `[`/`[[` with
     the held values rather than re-evaluating the call (a promise may
     rebind `x`).
  3. Still +3.9%: each `m[[i, j]] <- f(i, j)` iteration moved `m` into a VM
     slot (`own_var`: 3 frame lookups) and the call's flush wrote it back
     (`set_var` -> `Interp::define`: 2 more lookups, ~375 instructions).
     `own_var` now takes a plain binding in one lookup and `set_var`
     overwrites any plain non-function binding in place.
- tests: `tests/r/subset/matrix-elt2.R`, `tests/r/subset/index-promises.R`
  (order of evaluation, promises rebinding `x`, missing indices, dispatch
  with unforced promises, defaults), recorded with GNU R 4.5.0; conformance
  (all tiers) passes; fastr subsetting/functions slices unchanged
  (3138/3225 before and after).
- result: geomean 0.730 -> 0.705, faster than GNU R on 50 -> 52; 16 benchmarks faster,
  none slower: `matrixFor` -81.2% (5.45 -> 1.03), `nbody_naive` -35.1%
  (2.05 -> 1.33), `fastaredux_naive` -13.0%, `fasta_naive` -10.8%,
  `fasta_naive_2` -8.9%, `spectralnorm`/`spectralnorm_naive` -2.7%,
  `fastaredux` -2.4%, `mandelbrot_naive_ascii` -2.2%, `fasta*` -1.4..-1.7%,
  `vectorFor` -1.5%, `profiler-shared` -1.4%, `spectralnorm_alt_3` -1.4%,
  `Storage` -1.0%; kept.

## 2026-10-28: baseline (main 10d0a81)

instructions: robigo/GNU R geometric mean 0.730 over 74 correct programs; faster on 50
not counted (4): rbench/shootout/fannkuchredux, rbench/shootout/fannkuchredux_naive, rbench/shootout/mandelbrot_ascii, rbench/shootout/spectralnorm_math

Biggest losers: `simple/matrixFor` 5.45 (`x[[i, j]] <- 5` is not a VM
op), `nbody_naive` 2.05, the `spectralnorm` family 1.4-1.9, `fasta*`
1.1-1.3, `mandelbrot_naive_ascii` 1.30, `convolution_slow` 1.20, `fib`
1.11, `vectorFor`/`vectorWhile` 1.1, `binarytrees` 1.06.

Wrong output (bugs outside the VM, to look at): `fannkuchredux`,
`fannkuchredux_naive`, `mandelbrot_ascii`, `spectralnorm_math`.

<details><summary>full table</summary>

| benchmark | robigo instr | GNU R instr | ratio | |
|---|---|---|---|---|
| rbench/awf/Bounce | 707,305,990 | 825,271,903 | 0.86 |  |
| rbench/awf/Bounce_nonames | 679,454,447 | 883,548,558 | 0.77 |  |
| rbench/awf/Bounce_nonames_simple | 582,515,962 | 798,980,289 | 0.73 |  |
| rbench/awf/Mandelbrot | 295,906,518 | 497,760,722 | 0.59 |  |
| rbench/awf/Storage | 698,893,887 | 748,363,444 | 0.93 |  |
| rbench/real/convolution | 402,275,920 | 1,561,159,022 | 0.26 |  |
| rbench/real/convolution_slow | 2,166,599,480 | 1,799,232,018 | 1.20 |  |
| rbench/real/convolution_v | 729,414,303 | 733,969,146 | 0.99 |  |
| rbench/real/flexclust | 5,314,580,281 | 5,183,386,714 | 1.03 |  |
| rbench/real/flexclust_no_s4 | 5,764,701,927 | 5,796,739,300 | 0.99 |  |
| rbench/real/volcano | 286,146,047 | 531,117,525 | 0.54 |  |
| rbench/shootout/binarytrees | 503,998,184 | 687,255,881 | 0.73 |  |
| rbench/shootout/binarytrees_2 | 498,394,185 | 668,650,263 | 0.75 |  |
| rbench/shootout/binarytrees_naive | 519,418,949 | 664,387,524 | 0.78 |  |
| rbench/shootout/fannkuchredux | 285,911,305 | 730,170,002 | 0.39 | wrong output |
| rbench/shootout/fannkuchredux_naive | 285,923,859 | 651,918,956 | 0.44 | wrong output |
| rbench/shootout/fasta | 1,101,779,263 | 901,149,839 | 1.22 |  |
| rbench/shootout/fasta_2 | 1,232,021,218 | 967,608,768 | 1.27 |  |
| rbench/shootout/fasta_3 | 1,086,604,603 | 948,379,632 | 1.15 |  |
| rbench/shootout/fasta_naive | 1,076,580,567 | 961,751,152 | 1.12 |  |
| rbench/shootout/fasta_naive_2 | 924,936,200 | 952,761,439 | 0.97 |  |
| rbench/shootout/fastaredux | 1,210,266,893 | 960,732,802 | 1.26 |  |
| rbench/shootout/fastaredux_naive | 1,306,106,717 | 1,142,622,908 | 1.14 |  |
| rbench/shootout/knucleotide | 286,479,389 | 466,297,855 | 0.61 |  |
| rbench/shootout/knucleotide_brute | 286,742,503 | 463,861,957 | 0.62 |  |
| rbench/shootout/knucleotide_brute_2 | 286,515,075 | 463,693,649 | 0.62 |  |
| rbench/shootout/knucleotide_brute_3 | 286,530,760 | 463,764,366 | 0.62 |  |
| rbench/shootout/mandelbrot_ascii | 797,971,912 | 754,110,227 | 1.06 | wrong output |
| rbench/shootout/mandelbrot_naive_ascii | 1,451,992,303 | 1,116,959,646 | 1.30 |  |
| rbench/shootout/mandelbrot_noout | 995,174,926 | 847,875,611 | 1.17 |  |
| rbench/shootout/mandelbrot_noout_naive | 875,022,364 | 815,759,885 | 1.07 |  |
| rbench/shootout/nbody | 861,891,630 | 1,020,744,613 | 0.84 |  |
| rbench/shootout/nbody_2 | 976,928,304 | 1,039,116,024 | 0.94 |  |
| rbench/shootout/nbody_3 | 1,085,631,461 | 1,120,873,749 | 0.97 |  |
| rbench/shootout/nbody_naive | 2,776,987,488 | 1,355,419,139 | 2.05 |  |
| rbench/shootout/nbody_naive_2 | 1,386,608,936 | 1,290,535,487 | 1.07 |  |
| rbench/shootout/pidigits | 484,250,786 | 1,517,557,283 | 0.32 |  |
| rbench/shootout/regexdna | 286,037,950 | 426,175,535 | 0.67 |  |
| rbench/shootout/reversecomplement | 286,201,864 | 409,242,964 | 0.70 |  |
| rbench/shootout/reversecomplement_2 | 286,220,852 | 411,614,504 | 0.70 |  |
| rbench/shootout/reversecomplement_naive | 286,242,062 | 420,178,082 | 0.68 |  |
| rbench/shootout/spectralnorm | 2,697,507,200 | 1,443,529,040 | 1.87 |  |
| rbench/shootout/spectralnorm_alt | 2,307,836,945 | 1,235,707,101 | 1.87 |  |
| rbench/shootout/spectralnorm_alt_2 | 999,052,302 | 1,653,754,958 | 0.60 |  |
| rbench/shootout/spectralnorm_alt_3 | 2,333,916,053 | 1,631,301,809 | 1.43 |  |
| rbench/shootout/spectralnorm_math | 372,368,497 | 851,076,734 | 0.44 | wrong output |
| rbench/shootout/spectralnorm_naive | 2,717,441,503 | 1,740,930,666 | 1.56 |  |
| rbench/simple/bytecodes | 303,077,343 | 1,421,897,592 | 0.21 |  |
| rbench/simple/emptyFor | 371,559,341 | 1,354,110,092 | 0.27 |  |
| rbench/simple/emptyWhile | 321,173,773 | 1,581,987,531 | 0.20 |  |
| rbench/simple/lapply | 391,544,168 | 796,516,514 | 0.49 |  |
| rbench/simple/lapplyDots | 796,861,777 | 794,005,790 | 1.00 |  |
| rbench/simple/listFor | 378,475,666 | 949,971,642 | 0.40 |  |
| rbench/simple/listWhile | 489,045,062 | 1,247,892,405 | 0.39 |  |
| rbench/simple/matrixFor | 4,172,266,913 | 765,082,091 | 5.45 |  |
| rbench/simple/profiler-microbenchmark | 881,044,075 | 1,174,933,303 | 0.75 |  |
| rbench/simple/profiler-rsa | 575,551,002 | 1,104,080,689 | 0.52 |  |
| rbench/simple/profiler-shared | 6,620,471,296 | 6,935,667,565 | 0.95 |  |
| rbench/simple/scalarFor | 307,405,882 | 1,205,440,767 | 0.26 |  |
| rbench/simple/scalarWhile | 310,290,680 | 1,650,537,118 | 0.19 |  |
| rbench/simple/superWhile | 1,334,951,603 | 1,321,688,523 | 1.01 |  |
| rbench/simple/vectorFor | 1,524,401,065 | 1,377,027,569 | 1.11 |  |
| rbench/simple/vectorWhile | 1,653,874,388 | 1,516,542,769 | 1.09 |  |
| robigo/apply | 947,051,802 | 2,009,338,036 | 0.47 |  |
| robigo/binarytrees | 1,458,896,699 | 1,378,708,855 | 1.06 |  |
| robigo/csv | 1,006,606,203 | 1,542,426,146 | 0.65 |  |
| robigo/dataframe | 734,737,128 | 1,552,322,662 | 0.47 |  |
| robigo/fannkuch | 1,327,026,361 | 2,015,929,684 | 0.66 |  |
| robigo/fib | 1,202,226,293 | 1,079,788,250 | 1.11 |  |
| robigo/lm | 761,923,212 | 935,747,244 | 0.81 |  |
| robigo/mandelbrot | 336,450,739 | 983,589,471 | 0.34 |  |
| robigo/matrix | 756,837,353 | 1,410,574,442 | 0.54 |  |
| robigo/nbody | 488,373,385 | 2,632,988,973 | 0.19 |  |
| robigo/queens | 413,018,594 | 507,937,172 | 0.81 |  |
| robigo/spectralnorm | 354,145,940 | 2,641,067,067 | 0.13 |  |
| robigo/strings | 876,116,603 | 1,589,076,332 | 0.55 |  |
| robigo/vecarith | 4,464,965,026 | 5,409,460,545 | 0.83 |  |
| robigo/vecops | 2,539,798,129 | 4,015,961,152 | 0.63 |  |

</details>
