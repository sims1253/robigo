# perf regression of 2026-10-28: bisect, root causes, fix

Metric: callgrind instructions (`scripts/perf.py run --metric instr
--runtime robigo`), measured as in `perf-vm.md` § Setup notes: robigo
built and run in the `robigo-oracle:4.5.0` image (`debian:trixie`,
`r-base-core 4.5.0-3`), with a private `CARGO_TARGET_DIR`
(`~/.cache/robigo-bisect-target`) and a separate source worktree for the
candidates. Both are needed: building another checkout into the same target
directory linked stale `robigo_syntax` artifacts (cargo saw the same
packages).

## Problem

Between `37a5f96` and `ad59f6f`, `main` got slower: `dataframe` 705M ->
1,271M instructions, `mandelbrot_noout` +44%, `mandelbrot_ascii` +37%,
most other benchmarks about 1-2%. #9 and #29 were merged with stale CI, so
the 0.5% perf gate never compared them with their real base.

## Bisect (first-parent merges)

| commit | mandelbrot_ascii | mandelbrot_noout | shootout/nbody | dataframe | fib | robigo/nbody |
|---|---|---|---|---|---|---|
| 37a5f96 (good) | 756.3M | 950.0M | 831.9M | 705.4M | 1,190.3M | 452.5M |
| 80aa5d0 (#29) | 765.5M | 965.6M | 841.4M | **1,270.6M** | 1,170.2M | 461.6M |
| 383e95c (#9) | **1,037.1M** | **1,365.9M** | 842.2M | 1,270.6M | 1,170.2M | 461.5M |
| ad59f6f (#39) | 1,037.1M | 1,366.0M | 842.2M | 1,270.6M | 1,170.3M | 461.6M |
| 0e5d94e (main) | 1,037.3M | 1,366.1M | 842.4M | 1,270.8M | 1,170.4M | 461.7M |

#35 and #38 change no runtime code (corpora, CI). The ~9M added to every
benchmark at #29 is startup, with utils and stats loaded (`invisible(1)`:
249.6M -> 258.9M; with `R_DEFAULT_PACKAGES=NULL`, 127.7M at both); within
#29 it is `e2bde9c` (49c071a 280.8M -> e2bde9c 290.6M with utils+stats).

## Root causes

1. **#29, `e2bde9c`, collation** (dataframe +565M): `sort::scollate` became
   `if collation_is_c() { a.cmp(b) } else { options::scollate(a, b) }`.
   `collation_is_c()` called `setlocale(LC_COLLATE, NULL)` and converted the
   name with `from_utf8_lossy` on every comparison, and `options::scollate`
   collected both strings' weights into two `Vec<(u32, u32)>`, finding each
   character in the punctuation string with a linear search (`memchr`). In
   C.UTF-8 every comparison takes the second branch. callgrind diff, good ->
   bad: memchr +154M, `Vec<(u32,u32)>` collect +81M, `primary_weight` +73M,
   `Utf8Chunks` +39M, mi_free/malloc +70M, `collation_is_c` +30M,
   `setlocale` +16M, `scollate` +32M. dataframe's `order(df$g, -df$x)` makes
   about 1.7M comparisons.
2. **#29, `e2bde9c`, startup** (+9.9M on everything with utils): not a
   regression. Before, utils' `.onLoad` -> `.get_repositories()` failed
   inside a `tryCatch` ("there is no .Internal function 'file.info'").
   Now it reads the repositories file with `read.delim`, as GNU R does
   (the same 7x6 data frame; `getOption("repos")` is `@CRAN@` either way).
   The extra instructions are the lazy-load fetch and run of read.table and
   friends. Kept.
3. **#9, complex `*`** (mandelbrot +400M): `cmul` ported libgcc's
   `__muldc3` bit for bit as a plain pass, plus, whenever the result had a
   NaN, a second pass with x86-64 SSE NaN rules emulated in software (an
   `is_nan` test and an out-of-line `propagate` per operation). Escaping
   Mandelbrot points overflow to Inf and then NaN, so most of the 3.4M
   products took both passes and the infinity-recovery checks. callgrind:
   `cmul_impl<ExactArith>` +281M, `cmul` +150M, `sse::propagate` +62M,
   `arith_binary` +69M; the old inline `complex_op` -161M.

## Fix

- `collation_is_c` caches its answer in a thread-local until
  `Sys.setlocale` (R: util.c:Scollate's `collationLocaleSet`, reset by
  platform.c:do_setlocale -> `resetICUcollator`), with an inlined fast path.
  `options::scollate` gets a 128-entry ASCII weight table and compares ASCII
  strings in one pass over their bytes: the first primary difference wins,
  then the length, then the first tertiary difference, then the bytes. A
  non-ASCII byte at the first difference falls back to the per-character
  definition. A unit test checks the fast path against the definition on
  every pair of the 6,175 strings of up to three characters over an
  18-character alphabet with non-ASCII characters.
- `do_order`: when every key is logical, integer, double, complex or
  character, no comparison can fail, so std's stable `sort_by` replaces the
  recursive merge sort (which copied both halves at every merge), and
  character keys compare as their dense ranks among the distinct strings
  in collation order. That is exact because `scollate` is a total order
  that ties only identical strings, and a stable sort's result is
  determined by the comparisons. Other key types keep the general path and
  its errors (`unimplemented type 'list' in 'listgreater'`).
- complex: on x86-64, `sse::{add,sub,mul,div}` are the `addsd`/`subsd`/
  `mulsd`/`divsd` instructions (inline assembly, so LLVM cannot commute or
  constant-fold them). They are exactly the semantics that were being
  emulated, so `cmul` and `cdiv` run once with them. `__muldc3`'s infinity
  recovery is a cold function behind a test for an infinite operand or
  product (it changes the result only then). Other targets keep the
  two-pass scheme. The C library oracle test (16k products and quotients,
  21k NaN payload cases in total) still matches bit for bit in the
  container.
- `arith_binary`'s complex branch borrows complex operands (it copied
  both) and uses the real branch's recycling kernels (`real_kernel!`)
  instead of `i % nx`, `i % ny` and `push` per element.
- `Frame::get`/`get_mut` and `arith::real_op`/`int_to_real` are
  `#[inline]`. Release builds use 16 codegen units without LTO, so a
  non-`#[inline]` function is inlined only into callers in its own unit,
  and the changes above moved unit boundaries: `Frame::get` stopped being
  inlined into the interpreter's lookups (fib +0.85%, shootout/nbody +0.5%
  against 0e5d94e), and after rebasing on #44, `real_op` stopped being
  inlined into the VM's `arith_fast` (1.3M calls in vectorWhile: +0.6%).
  Variants tried on main 1696902 (full suite, robigo instructions):
  - without `Frame::get` inline: superWhile +1.35%, nbody_naive +0.59%
    (`Frame::get` called out of line from `eval_sym`, `define`,
    `super_assign`); rejected.
  - with it: superWhile -0.73% but vectorFor +0.50%. That is register
    allocation in `vm::run_with` (about 4 more instructions per
    iteration on lines without line info; same calls), in perf-vm's code.
    Dropping the `get_mut` or `int_to_real` attribute changes nothing;
    kept.
  A release profile with `codegen-units = 1` (or LTO) would remove this
  fragility; that is a repository-wide build-time decision, so it is not
  made here.

## Steps (6 benchmarks)

| step | mandelbrot_ascii | mandelbrot_noout | shootout/nbody | dataframe | fib | robigo/nbody |
|---|---|---|---|---|---|---|
| main 0e5d94e | 1,037.3M | 1,366.1M | 842.4M | 1,270.8M | 1,170.4M | 461.7M |
| + cached locale, no-alloc scollate, SSE asm, complex kernels | 538.7M | 626.7M | 846.6M | 776.7M | 1,180.3M | 462.4M |
| + one-pass ASCII scollate, inlined cache test | 538.7M | 626.7M | 846.7M | 758.8M | 1,180.3M | 462.4M |
| + `#[inline]` Frame::get | 536.6M | 624.5M | 839.2M | 757.1M | 1,166.2M | 460.8M |
| + order by collation rank, std stable sort | 536.9M | 624.9M | 839.7M | 663.0M | 1,166.5M | 461.1M |

## Result (all benchmarks, rebased on main 1696902)

`scripts/perf.py compare` against main 1696902 (after #44): **20 faster,
0 slower** by more than 0.5% (geomean of robigo instructions this PR / main
0.966; the closest is `vectorFor` at +0.499%): `mandelbrot_noout` -54.0%,
`mandelbrot_ascii` -48.0%, `dataframe` -47.6%, `mandelbrot_noout_naive`
-24.7%, `mandelbrot_naive_ascii` -4.7%, `spectralnorm_naive` -2.3%,
`spectralnorm` -1.5%, `profiler-shared` -1.1%, and 12 more between -0.5%
and -0.9%. Before the rebase, against 963b3cf: 8 faster, 0 slower. CI's perf job on
this PR rebased on 414ae44 (#45): 21 faster, 0 slower, 0 broken.

Against 37a5f96, the regressed benchmarks are now below their old counts:
`dataframe` 705.3M -> 667.2M (-5.4%), `mandelbrot_noout` 950.0M -> 628.6M
(-33.8%), `mandelbrot_ascii` 756.3M -> 540.8M (-28.5%). The rest is still
1-5% above 37a5f96. On small programs that is a near-constant 12-14M, all
startup with utils+stats (`invisible(1)`):

| | startup |
|---|---|
| 37a5f96 | 249.6M |
| 80aa5d0 (#29: utils' `.onLoad` reads the repositories file, as GNU R) | 258.9M |
| 0e5d94e | 259.2M |
| 963b3cf (#32, #41, #42, merged after the window) | 263.1M |
| this PR on 963b3cf | 262.1M |

So none of that is overhead from #9 or #29. About 90% of #29's 9.3M is the
lazy-load fetch (zlib inflate and unserialize) of read.table and the
functions it calls. Making lazy loading cheaper would bring it back; that
belongs to the startup track.

<details><summary>full table (robigo instructions)</summary>

| benchmark | 37a5f96 | main 1696902 | this PR | vs main | vs 37a5f96 |
|---|---|---|---|---|---|
| rbench/awf/Bounce | 678,561,625 | 693,750,143 | 689,407,546 | -0.63% | +1.60% |
| rbench/awf/Bounce_nonames | 638,162,161 | 652,664,201 | 650,319,065 | -0.36% | +1.90% |
| rbench/awf/Bounce_nonames_simple | 544,311,525 | 556,637,324 | 554,609,082 | -0.36% | +1.89% |
| rbench/awf/Mandelbrot | 265,025,513 | 277,825,649 | 276,817,400 | -0.36% | +4.45% |
| rbench/awf/Storage | 655,694,651 | 668,117,943 | 665,934,978 | -0.33% | +1.56% |
| rbench/real/convolution | 371,382,909 | 385,072,608 | 384,159,858 | -0.24% | +3.44% |
| rbench/real/convolution_slow | 2,108,717,880 | 2,123,533,740 | 2,115,773,665 | -0.37% | +0.33% |
| rbench/real/convolution_v | 698,544,186 | 712,081,312 | 711,250,290 | -0.12% | +1.82% |
| rbench/real/flexclust | 4,783,611,542 | 4,776,621,968 | 4,773,232,478 | -0.07% | -0.22% |
| rbench/real/flexclust_no_s4 | 5,234,614,123 | 5,223,950,774 | 5,218,100,664 | -0.11% | -0.32% |
| rbench/real/volcano | 255,169,355 | 268,852,864 | 267,979,799 | -0.32% | +5.02% |
| rbench/shootout/binarytrees | 470,126,063 | 477,613,816 | 476,242,124 | -0.29% | +1.30% |
| rbench/shootout/binarytrees_2 | 465,986,559 | 476,076,450 | 474,530,935 | -0.32% | +1.83% |
| rbench/shootout/binarytrees_naive | 464,620,812 | 474,720,511 | 473,198,249 | -0.32% | +1.85% |
| rbench/shootout/fannkuchredux | 255,023,197 | 268,743,356 | 267,827,488 | -0.34% | +5.02% |
| rbench/shootout/fannkuchredux_naive | 255,032,745 | 268,735,676 | 267,817,519 | -0.34% | +5.01% |
| rbench/shootout/fasta | 1,047,931,749 | 873,528,700 | 869,929,069 | -0.41% | -16.99% |
| rbench/shootout/fasta_2 | 1,175,826,566 | 992,817,939 | 988,891,782 | -0.40% | -15.90% |
| rbench/shootout/fasta_3 | 1,033,636,194 | 894,073,109 | 890,578,370 | -0.39% | -13.84% |
| rbench/shootout/fasta_naive | 1,022,011,476 | 910,390,580 | 906,532,813 | -0.42% | -11.30% |
| rbench/shootout/fasta_naive_2 | 802,546,254 | 809,942,364 | 805,284,863 | -0.58% | +0.34% |
| rbench/shootout/fastaredux | 1,153,558,066 | 917,675,523 | 912,793,482 | -0.53% | -20.87% |
| rbench/shootout/fastaredux_naive | 1,082,759,344 | 1,094,241,398 | 1,088,275,434 | -0.55% | +0.51% |
| rbench/shootout/knucleotide | 255,599,337 | 269,298,695 | 268,331,402 | -0.36% | +4.98% |
| rbench/shootout/knucleotide_brute | 255,755,637 | 269,433,126 | 268,489,141 | -0.35% | +4.98% |
| rbench/shootout/knucleotide_brute_2 | 255,541,820 | 269,271,603 | 268,320,227 | -0.35% | +5.00% |
| rbench/shootout/knucleotide_brute_3 | 255,524,143 | 269,269,000 | 268,361,892 | -0.34% | +5.02% |
| rbench/shootout/mandelbrot_ascii | 756,251,838 | 1,039,337,413 | 540,755,575 | -47.97% | -28.50% |
| rbench/shootout/mandelbrot_naive_ascii | 1,356,693,720 | 1,367,271,737 | 1,303,482,405 | -4.67% | -3.92% |
| rbench/shootout/mandelbrot_noout | 950,008,329 | 1,366,966,576 | 628,647,857 | -54.01% | -33.83% |
| rbench/shootout/mandelbrot_noout_naive | 845,114,422 | 976,731,615 | 735,573,816 | -24.69% | -12.96% |
| rbench/shootout/nbody | 831,913,778 | 850,733,057 | 845,358,895 | -0.63% | +1.62% |
| rbench/shootout/nbody_2 | 942,868,561 | 957,000,109 | 958,105,710 | +0.12% | +1.62% |
| rbench/shootout/nbody_3 | 1,047,943,892 | 1,061,694,016 | 1,063,037,855 | +0.13% | +1.44% |
| rbench/shootout/nbody_naive | 1,792,192,598 | 1,789,332,033 | 1,781,727,672 | -0.42% | -0.58% |
| rbench/shootout/nbody_naive_2 | 1,351,937,913 | 1,365,732,365 | 1,367,986,332 | +0.17% | +1.19% |
| rbench/shootout/pidigits | 452,993,996 | 465,092,902 | 462,704,109 | -0.51% | +2.14% |
| rbench/shootout/regexdna | 255,222,387 | 268,861,081 | 267,977,702 | -0.33% | +5.00% |
| rbench/shootout/reversecomplement | 255,380,109 | 269,001,214 | 268,114,592 | -0.33% | +4.99% |
| rbench/shootout/reversecomplement_2 | 255,420,156 | 269,009,393 | 268,104,490 | -0.34% | +4.97% |
| rbench/shootout/reversecomplement_naive | 255,434,234 | 269,036,129 | 268,121,005 | -0.34% | +4.97% |
| rbench/shootout/spectralnorm | 771,912,753 | 785,865,935 | 774,488,408 | -1.45% | +0.33% |
| rbench/shootout/spectralnorm_alt | 2,276,967,796 | 1,012,756,451 | 1,011,873,739 | -0.09% | -55.56% |
| rbench/shootout/spectralnorm_alt_2 | 968,160,185 | 859,755,086 | 858,760,813 | -0.12% | -11.30% |
| rbench/shootout/spectralnorm_alt_3 | 2,188,821,989 | 1,345,509,414 | 1,334,104,583 | -0.85% | -39.05% |
| rbench/shootout/spectralnorm_math | 341,531,462 | 355,371,225 | 353,810,358 | -0.44% | +3.60% |
| rbench/shootout/spectralnorm_naive | 1,045,410,467 | 1,059,336,283 | 1,035,406,914 | -2.26% | -0.96% |
| rbench/simple/bytecodes | 272,218,953 | 285,883,535 | 284,963,655 | -0.32% | +4.68% |
| rbench/simple/emptyFor | 340,755,061 | 354,523,639 | 353,573,949 | -0.27% | +3.76% |
| rbench/simple/emptyWhile | 290,399,827 | 304,072,518 | 303,112,218 | -0.32% | +4.38% |
| rbench/simple/lapply | 351,812,141 | 365,480,176 | 363,421,173 | -0.56% | +3.30% |
| rbench/simple/lapplyDots | 763,840,207 | 765,616,505 | 761,726,243 | -0.51% | -0.28% |
| rbench/simple/listFor | 346,490,079 | 360,509,958 | 360,071,496 | -0.12% | +3.92% |
| rbench/simple/listWhile | 458,738,562 | 470,750,604 | 469,010,245 | -0.37% | +2.24% |
| rbench/simple/matrixFor | 752,811,572 | 768,301,407 | 768,011,983 | -0.04% | +2.02% |
| rbench/simple/profiler-microbenchmark | 780,497,741 | 790,769,228 | 789,819,429 | -0.12% | +1.19% |
| rbench/simple/profiler-rsa | 537,835,876 | 548,172,312 | 544,452,934 | -0.68% | +1.23% |
| rbench/simple/profiler-shared | 6,729,961,721 | 6,194,749,597 | 6,128,706,653 | -1.07% | -8.93% |
| rbench/simple/scalarFor | 276,547,471 | 290,271,435 | 289,351,002 | -0.32% | +4.63% |
| rbench/simple/scalarWhile | 279,437,974 | 293,194,720 | 292,224,096 | -0.33% | +4.58% |
| rbench/simple/superWhile | 1,314,103,002 | 1,321,561,543 | 1,311,977,883 | -0.73% | -0.16% |
| rbench/simple/vectorFor | 1,472,539,066 | 1,492,572,002 | 1,500,018,280 | +0.50% | +1.87% |
| rbench/simple/vectorWhile | 1,625,701,029 | 1,626,169,244 | 1,619,908,624 | -0.38% | -0.36% |
| robigo/apply | 896,780,117 | 910,806,478 | 902,821,190 | -0.88% | +0.67% |
| robigo/binarytrees | 1,467,117,508 | 1,377,400,781 | 1,374,861,303 | -0.18% | -6.29% |
| robigo/csv | 935,115,777 | 942,723,404 | 940,962,313 | -0.19% | +0.63% |
| robigo/dataframe | 705,317,235 | 1,273,035,792 | 667,174,555 | -47.59% | -5.41% |
| robigo/fannkuch | 1,289,210,621 | 1,301,015,858 | 1,297,744,850 | -0.25% | +0.66% |
| robigo/fib | 1,190,115,178 | 1,056,117,618 | 1,052,413,498 | -0.35% | -11.57% |
| robigo/lm | 733,677,197 | 744,165,288 | 743,415,841 | -0.10% | +1.33% |
| robigo/mandelbrot | 305,277,621 | 319,080,538 | 318,100,849 | -0.31% | +4.20% |
| robigo/matrix | 725,939,173 | 740,374,398 | 739,444,020 | -0.13% | +1.86% |
| robigo/nbody | 452,501,627 | 466,021,440 | 465,078,084 | -0.20% | +2.78% |
| robigo/queens | 381,785,376 | 389,995,521 | 388,769,642 | -0.31% | +1.83% |
| robigo/spectralnorm | 323,200,694 | 336,798,326 | 335,891,619 | -0.27% | +3.93% |
| robigo/strings | 846,618,338 | 859,649,089 | 857,533,969 | -0.25% | +1.29% |
| robigo/vecarith | 4,433,938,385 | 4,462,769,911 | 4,461,855,669 | -0.02% | +0.63% |
| robigo/vecops | 2,508,782,900 | 2,522,441,737 | 2,521,546,942 | -0.04% | +0.51% |

</details>

## Verification

- `cargo fmt --check` and `cargo test --release --workspace` in the
  container (unit tests including the complex C library oracle and the new
  `scollate_matches_weight_sequences`, parser oracle, conformance in all
  three tiers: interpreter, VM, VM+JIT). New conformance scripts
  `tests/r/vec/order-strings.R` (collation order, NA placement,
  decreasing, several keys, ties, `Sys.setlocale("LC_COLLATE", "C")` and
  back, the list-key error) and `tests/r/math/complex-vector-arith.R`
  (recycling, Inf/NaN, overflow to NaN, attributes, `%%` error), recorded
  with GNU R 4.5.0 in the container and added to the ratchet.
  order-strings is ASCII only: robigo sorts non-ASCII strings by code
  point, not with ICU (unchanged; main gives the same output).
- FastR, the conformance-values slice of #9 plus sort/order/rank/xtfrm,
  complex, print/format and file/Sys tests
  (`TestSimpleValues|TestSimpleArithmetic|TestSimpleComparison|TestBuiltin_(operators|log|asvector|asraw|intToBits|order|sort|rank|xtfrm|complex|Conj|Mod|Arg|Re|Im|polyroot|fileinfo|fileaccess|print|format|strtoi|Sys)`,
  10,988 cases) in the container: main 963b3cf 10,638 -> 10,648, none lost
  (and CI's corpus jobs on this PR pass).
  The 10 gained are NaN-payload cases of complex `+`, whose `re` part is a
  plain Rust `+` and depends on LLVM's operand order; the new kernel
  happens to emit GNU R's order. The ratchet's 15-20 "regressions" in this
  slice (complex `*`/`%*%` NaN payloads, e.g. `(1i+NA)*(0/0)`, and three
  others) fail the same way with CI's own binary of main and are not
  failures in CI's corpus job.
- Benchmark output against GNU R (`perf.py run --metric time --runtime
  both`): the same 3 wrong programs as main (`fannkuchredux`,
  `fannkuchredux_naive`, `spectralnorm_math`).
