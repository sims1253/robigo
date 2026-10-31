# Benchmarks: Full Suite Hill-Climb

Metric: `scripts/perf.py run --metric instr`, measuring instruction counts against GNU R 4.5.0 with callgrind in the same run (`--runtime both`). Gate: `scripts/perf.py compare` (0 slower above 0.5% threshold) and conformance tests.

## Round 1: Hunt the biggest absolute gaps

- Base: `main` at `c3bb38a8`.
- Environment: Host Linux x86_64, pinned GNU R 4.5.0 (`~/.local/opt/R-4.5.0`), valgrind 3.25.1.
- Full suite baseline: 73 correct programs measured against GNU R 4.5.0. Overall robigo/GNU R geometric mean: **0.481x** (robigo faster on all 73).
- Top three targets by absolute instruction gap (`robigo_instr - gnur_instr` descending, where robigo is closest to GNU R):
  1. `rbench/shootout/nbody_naive_2`: diff **-36,753,803** (robigo 1,256,828,301 vs GNU R 1,293,582,104, ratio 0.972x)
  2. `rbench/real/convolution_v`: diff **-96,502,055** (robigo 640,829,588 vs GNU R 737,331,643, ratio 0.869x)
  3. `rbench/simple/lapplyDots`: diff **-107,356,909** (robigo 690,351,965 vs GNU R 797,708,874, ratio 0.865x)

### Baseline Ranking (73 correct programs, sorted by robigo - gnur gap)

| Rank | Benchmark | robigo instr | GNU R instr | Diff (robigo - gnur) | Ratio |
|---:|---|---:|---:|---:|---:|
| 1 | `rbench/shootout/nbody_naive_2` | 1,256,828,301 | 1,293,582,104 | -36,753,803 | 0.972 |
| 2 | `rbench/real/convolution_v` | 640,829,588 | 737,331,643 | -96,502,055 | 0.869 |
| 3 | `rbench/simple/lapplyDots` | 690,351,965 | 797,708,874 | -107,356,909 | 0.865 |
| 4 | `robigo/fib` | 956,125,417 | 1,084,692,461 | -128,567,044 | 0.881 |
| 5 | `robigo/binarytrees` | 1,251,069,519 | 1,383,650,034 | -132,580,515 | 0.904 |
| 6 | `rbench/shootout/mandelbrot_noout_naive` | 625,975,232 | 778,979,605 | -153,004,373 | 0.804 |
| 7 | `rbench/shootout/nbody_3` | 964,220,433 | 1,122,688,447 | -158,468,014 | 0.859 |
| 8 | `rbench/shootout/fannkuchredux` | 568,589,746 | 733,539,697 | -164,949,951 | 0.775 |
| 9 | `rbench/shootout/nbody_2` | 864,727,208 | 1,041,095,498 | -176,368,290 | 0.831 |
| 10 | `rbench/awf/Storage` | 554,241,588 | 752,319,525 | -198,077,937 | 0.737 |
| 11 | `rbench/awf/Bounce` | 617,665,751 | 828,683,935 | -211,018,184 | 0.745 |
| 12 | `rbench/shootout/binarytrees_naive` | 413,419,276 | 667,916,693 | -254,497,417 | 0.619 |
| 13 | `robigo/queens` | 256,901,150 | 511,437,068 | -254,535,918 | 0.502 |
| 14 | `rbench/shootout/binarytrees_2` | 414,443,143 | 672,078,131 | -257,634,988 | 0.617 |
| 15 | `robigo/lm` | 675,721,028 | 934,980,953 | -259,259,925 | 0.723 |
| 16 | `rbench/shootout/nbody` | 762,911,506 | 1,022,400,122 | -259,488,616 | 0.746 |
| 17 | `rbench/shootout/mandelbrot_naive_ascii` | 850,156,160 | 1,117,159,881 | -267,003,721 | 0.761 |
| 18 | `rbench/shootout/fasta_2` | 693,802,615 | 967,632,876 | -273,830,261 | 0.717 |
| 19 | `rbench/shootout/mandelbrot_ascii` | 482,072,455 | 756,220,101 | -274,147,646 | 0.637 |
| 20 | `rbench/shootout/binarytrees` | 414,221,293 | 690,762,850 | -276,541,557 | 0.600 |
| 21 | `rbench/awf/Mandelbrot` | 223,362,308 | 501,256,575 | -277,894,267 | 0.446 |
| 22 | `rbench/shootout/mandelbrot_noout` | 566,482,011 | 850,031,154 | -283,549,143 | 0.666 |
| 23 | `rbench/shootout/fasta_3` | 643,515,656 | 948,283,820 | -304,768,164 | 0.679 |
| 24 | `rbench/shootout/fasta` | 585,804,596 | 902,074,047 | -316,269,451 | 0.649 |
| 25 | `rbench/shootout/spectralnorm_math` | 537,894,840 | 854,285,762 | -316,390,922 | 0.630 |
| 26 | `rbench/awf/Bounce_nonames` | 567,280,778 | 887,026,772 | -319,745,994 | 0.640 |
| 27 | `rbench/shootout/spectralnorm_alt` | 918,392,537 | 1,239,090,103 | -320,697,566 | 0.741 |
| 28 | `rbench/awf/Bounce_nonames_simple` | 480,221,930 | 802,402,963 | -322,181,033 | 0.598 |
| 29 | `rbench/shootout/fannkuchredux_naive` | 336,718,059 | 655,207,010 | -318,488,951 | 0.514 |
| 30 | `robigo/s4-valid` | 612,613,498 | 961,184,302 | -348,570,804 | 0.637 |
| 31 | `robigo/s4-new` | 608,556,914 | 954,212,671 | -345,655,757 | 0.638 |
| 32 | `robigo/s4-slot-read` | 453,855,789 | 804,576,642 | -350,720,853 | 0.564 |
| 33 | `rbench/shootout/fasta_naive` | 629,903,586 | 965,389,767 | -335,486,181 | 0.652 |
| 34 | `rbench/shootout/fasta_naive_2` | 601,539,423 | 956,291,907 | -354,752,484 | 0.629 |
| 35 | `robigo/s4-slot-write` | 480,108,879 | 844,968,491 | -364,859,612 | 0.568 |
| 36 | `robigo/s4-generic` | 578,553,670 | 950,678,159 | -372,124,489 | 0.609 |
| 37 | `rbench/shootout/spectralnorm_alt_3` | 1,246,003,686 | 1,635,484,773 | -389,481,087 | 0.762 |
| 38 | `rbench/shootout/fastaredux` | 568,944,500 | 963,431,314 | -394,486,814 | 0.591 |
| 39 | `robigo/s4-portfolio` | 567,715,499 | 963,308,700 | -395,593,201 | 0.589 |
| 40 | `rbench/shootout/fastaredux_naive` | 714,585,938 | 1,146,322,073 | -431,736,135 | 0.623 |
| 41 | `rbench/simple/profiler-microbenchmark` | 719,847,846 | 1,179,304,935 | -459,457,089 | 0.610 |
| 42 | `rbench/simple/matrixFor` | 270,373,033 | 768,403,053 | -498,030,020 | 0.352 |
| 43 | `rbench/simple/lapply` | 297,488,291 | 801,175,110 | -503,686,819 | 0.371 |
| 44 | `robigo/csv` | 943,183,993 | 1,533,955,307 | -590,771,314 | 0.615 |
| 45 | `rbench/simple/profiler-rsa` | 487,691,976 | 1,107,488,091 | -619,796,115 | 0.440 |
| 46 | `rbench/simple/listFor` | 302,500,393 | 953,215,124 | -650,714,731 | 0.317 |
| 47 | `robigo/matrix` | 685,317,688 | 1,414,862,306 | -729,544,618 | 0.484 |
| 48 | `robigo/mandelbrot` | 253,857,523 | 987,150,907 | -733,293,384 | 0.257 |
| 49 | `rbench/shootout/nbody_naive` | 611,485,973 | 1,358,509,616 | -747,023,643 | 0.450 |
| 50 | `rbench/shootout/spectralnorm_naive` | 984,817,342 | 1,745,530,186 | -760,712,844 | 0.564 |
| 51 | `rbench/simple/superWhile` | 508,717,672 | 1,324,983,070 | -816,265,398 | 0.384 |
| 52 | `robigo/strings` | 761,921,813 | 1,579,865,328 | -817,943,515 | 0.482 |
| 53 | `robigo/fannkuch` | 1,191,345,393 | 2,019,324,468 | -827,979,075 | 0.590 |
| 54 | `rbench/simple/listWhile` | 409,018,754 | 1,251,123,161 | -842,104,407 | 0.327 |
| 55 | `robigo/dataframe` | 619,991,853 | 1,558,802,065 | -938,810,212 | 0.398 |
| 56 | `rbench/simple/scalarFor` | 236,469,133 | 1,208,747,132 | -972,277,999 | 0.196 |
| 57 | `rbench/shootout/spectralnorm_alt_2` | 665,973,860 | 1,657,037,779 | -991,063,919 | 0.402 |
| 58 | `rbench/simple/emptyFor` | 300,775,808 | 1,357,504,914 | -1,056,729,106 | 0.222 |
| 59 | `rbench/simple/vectorFor` | 284,904,287 | 1,380,338,669 | -1,095,434,382 | 0.206 |
| 60 | `rbench/shootout/pidigits` | 404,493,982 | 1,521,236,516 | -1,116,742,534 | 0.266 |
| 61 | `robigo/vecarith` | 4,036,032,969 | 5,152,878,088 | -1,116,845,119 | 0.783 |
| 62 | `rbench/simple/profiler-shared` | 5,777,032,165 | 6,948,021,738 | -1,170,989,573 | 0.831 |
| 63 | `rbench/shootout/spectralnorm` | 259,641,987 | 1,447,740,572 | -1,188,098,585 | 0.179 |
| 64 | `rbench/simple/bytecodes` | 229,217,444 | 1,425,230,231 | -1,196,012,787 | 0.161 |
| 65 | `robigo/apply` | 811,271,246 | 2,014,406,418 | -1,203,135,172 | 0.403 |
| 66 | `rbench/real/convolution` | 325,386,389 | 1,564,542,921 | -1,239,156,532 | 0.208 |
| 67 | `rbench/simple/vectorWhile` | 263,029,255 | 1,519,858,783 | -1,256,829,528 | 0.173 |
| 68 | `rbench/simple/emptyWhile` | 250,452,697 | 1,585,379,556 | -1,334,926,859 | 0.158 |
| 69 | `rbench/simple/scalarWhile` | 239,406,954 | 1,653,847,479 | -1,414,440,525 | 0.145 |
| 70 | `rbench/real/convolution_slow` | 362,055,438 | 1,802,854,744 | -1,440,799,306 | 0.201 |
| 71 | `robigo/vecops` | 2,433,601,613 | 4,021,379,217 | -1,587,777,604 | 0.605 |
| 72 | `robigo/nbody` | 373,818,416 | 2,636,362,194 | -2,262,543,778 | 0.142 |
| 73 | `robigo/spectralnorm` | 268,539,716 | 2,646,918,073 | -2,378,378,357 | 0.101 |

---

## Target 1: `rbench/shootout/nbody_naive_2`

### Profiling & Gap Attribution
Callgrind profiling of `rbench/shootout/nbody_naive_2` revealed heavy overhead in `advance()` inner loops:
1. `r_is_missing` in `crates/robigo-runtime/src/builtins/subset.rs`: was cloning `b.value` on every environment frame lookup in `eval_keep_missing`. Fixed to borrow `&b.value` directly and fast-path empty symbols `x[, i]`.
2. `dim_of` and `matrix_assign`: extracting matrix dimensions repeatedly cloned `Value` and attributes `s::DIM`. Added zero-copy `DimView` and `dim_view`.
3. Matrix column extraction `body_r[, i]`: was allocating intermediate row/column index vectors, collecting element-by-element, creating DIM attributes, and calling `drop_dims`. Added column slice fast-path copying contiguous slice directly.
4. Matrix column assignment `body_v[, i] <<- ...`: was allocating odometer structures. Added contiguous slice mutation fast-path.
5. Array slice assignment `drr[i, j, ] <- ...`: was allocating nested index vectors. Added strided in-place assignment fast path.
6. Binary arithmetic attribute checks: `arith_binary` was querying attributes when neither operand had attributes. Added fast-path bypass when `!x.has_attrs() && !y.has_attrs()`.

### Results
- Before: `1,256,828,301` instructions (GNU R: `1,293,582,104`, ratio 0.972x)
- After: `986,028,288` instructions (GNU R: `1,293,582,104`, ratio 0.762x)
- Delta: **-270,800,013 instructions (-21.55%)**
- Conformance: Output verified byte-identical against GNU R. Gate comparison: 0 regressions > 0.5%.

---

## Target 2: `rbench/real/convolution_v`

### Profiling & Gap Attribution
Callgrind profiling of `rbench/real/convolution_v` (`convolveV(1:583, 1:583)`) revealed:
1. `rbind` in `crates/robigo-runtime/src/builtins/array.rs`: Allocated a `Vec<(usize, usize)>` coordinate mapping of `rows * cols` entries (679,778 tuples = ~11 MB per call), scanned the vector with `.all()`, and invoked `gather_multi` which matched variants and indexed scalar-by-scalar. Attributed to ~175M instructions in callgrind profile.
2. `do_matrix` in `crates/robigo-runtime/src/builtins/array.rs`: Allocated `(0..n).map(|k| k % lendat).collect::<Vec<usize>>()` for `matrix(0, nx, ny)` and gathered element-by-element. Attributed to ~57.4M instructions in callgrind profile.
3. Dimension lookups: `dims_of` cloned `s::DIM` attributes (`Vec<i32>`) on every dimension check in `is_matrix`, `nrows`, and `ncols`.

### Optimizations
1. Implemented `fill_rbind` and `fill_cbind`: Replaced coordinate mapping and `gather_multi` with direct column-slice chunk copies (`copy_from_slice` / `clone_from_slice`) for matrices and unrolled indexing for vectors, matching GNU R's direct matrix fill loops in `bind.c`.
2. Implemented `repeat_to_len`: Fast scalar repetition (`vec![x; n]`), full-length clone, or slice extension loops for `do_matrix` (when `byrow == false`) and `do_array`, eliminating all index vector allocations.
3. Zero-copy dimension inspection: Switched `is_matrix`, `nrows`, `ncols`, `cbind`, and `rbind` to inspect `dims_slice` without allocating `Vec<i32>`.

### Results
- Before: `640,829,588` instructions (GNU R: `737,331,643`, ratio 0.869x)
- After: `297,511,256` instructions (GNU R: `737,331,643`, ratio 0.403x)
- Delta: **-343,318,332 instructions (-53.58%)**
- Conformance: Output verified byte-identical against GNU R. Gate comparison: 0 regressions > 0.5% (candidate is 53.57% faster).

---

## Target 3: `rbench/simple/lapplyDots`

### Profiling & Gap Attribution
Callgrind profiling of `rbench/simple/lapplyDots` (`execute(72236)` where `f <- function(x, y) x + 1.5 * y; lapply(x, f, y = a)`) revealed:
1. `inline_fun` in `crates/robigo-runtime/src/builtins/apply.rs` previously returned `None` whenever `dots` was non-empty (`rho.get_local(s::DOTS)` was not empty) or `c.formals.len() != 1`. This completely disabled fast inlining for any `lapply` call with `...` arguments.
2. Because inlining was aborted, every iteration (72,236 times) called `call_fun`:
   - Resolved `f` via `find_fun` dynamically walking the environment hierarchy on each iteration.
   - Allocated heap promises for `X[[i]]` and cloned `dots` on every iteration (`promise_args`).
   - Evaluated `X[[i]]` AST in `rho` via dynamic `[[` dispatch instead of direct indexing.

### Optimizations
1. Multi-argument and dots closure inlining in `inline_fun`: Extended `inline_fun` to support multi-argument pure inlinable closures with `...` arguments. Arguments from `dots` are matched to formals and evaluated/forced once prior to the loop.
2. `FastApply` specialization: Provided `FastApply::One(c)` for pure single-argument closures (zero allocation, stack slice) and `FastApply::Multi(fc)` for multi-argument closures with pre-evaluated invariant arguments.
3. Hoisted iteration loops in `do_lapply`: Hoisted branch selection outside the 72,236-iteration loop, eliminating per-element enum pattern matching and call overhead.

### Results
- Before: `690,351,965` instructions (GNU R: `797,708,874`, ratio 0.865x)
- After: `294,589,631` instructions (GNU R: `797,708,875`, ratio 0.369x)
- Delta: **-395,762,334 instructions (-57.33%)**
- Conformance: Output verified byte-identical against GNU R. Gate comparison: 0 regressions > 0.5% (both `lapply` and `lapplyDots` faster).



