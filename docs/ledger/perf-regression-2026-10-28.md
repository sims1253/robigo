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
- `Frame::get`/`get_mut` are `#[inline]`. After the changes above, LLVM
  stopped inlining `Frame::get` into `plain_lookup` and friends because it
  ended up in another codegen unit: fib +0.85%, shootout/nbody +0.5%
  against main, with no code of theirs touched.

## Steps (6 benchmarks)

| step | mandelbrot_ascii | mandelbrot_noout | shootout/nbody | dataframe | fib | robigo/nbody |
|---|---|---|---|---|---|---|
| main 0e5d94e | 1,037.3M | 1,366.1M | 842.4M | 1,270.8M | 1,170.4M | 461.7M |
| + cached locale, no-alloc scollate, SSE asm, complex kernels | 538.7M | 626.7M | 846.6M | 776.7M | 1,180.3M | 462.4M |
| + one-pass ASCII scollate, inlined cache test | 538.7M | 626.7M | 846.7M | 758.8M | 1,180.3M | 462.4M |
| + `#[inline]` Frame::get | 536.6M | 624.5M | 839.2M | 757.1M | 1,166.2M | 460.8M |
| + order by collation rank, std stable sort | 536.9M | 624.9M | 839.7M | 663.0M | 1,166.5M | 461.1M |

## Result (all benchmarks, rebased on main 963b3cf)

`scripts/perf.py compare` against main 963b3cf: **8 faster, 0 slower**
(geomean of robigo instructions this PR / main 0.967): `mandelbrot_noout`
-54.0%, `mandelbrot_ascii` -48.0%, `dataframe` -47.6%,
`mandelbrot_noout_naive` -24.7%, `mandelbrot_naive_ascii` -4.6%,
`profiler-shared` -0.95%, `superWhile` -0.94%, `Bounce` -0.61%.

Against 37a5f96, the regressed benchmarks are now below their old counts:
`dataframe` 705.3M -> 666.8M (-5.5%), `mandelbrot_noout` 950.0M -> 628.6M
(-33.8%), `mandelbrot_ascii` 756.3M -> 540.6M (-28.5%). The rest is still
1-5% above 37a5f96 (60 slower by more than 0.5%), a near-constant 12-14M
that is all startup with utils+stats (`invisible(1)`):

| | startup |
|---|---|
| 37a5f96 | 249.6M |
| 80aa5d0 (#29: utils' `.onLoad` reads the repositories file, as GNU R) | 258.9M |
| 0e5d94e | 259.2M |
| 963b3cf (#32, #41, #42, merged after the window) | 263.1M |
| this PR | 262.1M |

So none of that is overhead from #9 or #29. About 90% of #29's 9.3M is the
lazy-load fetch (zlib inflate and unserialize) of read.table and the
functions it calls. Making lazy loading cheaper would bring it back; that
belongs to the startup track.

<details><summary>full table (robigo instructions)</summary>

| benchmark | 37a5f96 | main 963b3cf | this PR | vs main | vs 37a5f96 |
|---|---|---|---|---|---|
| rbench/awf/Bounce | 678,561,625 | 693,021,287 | 688,814,658 | -0.61% | +1.51% |
| rbench/awf/Bounce_nonames | 638,162,161 | 651,617,369 | 649,331,706 | -0.35% | +1.75% |
| rbench/awf/Bounce_nonames_simple | 544,311,525 | 555,752,307 | 553,661,550 | -0.38% | +1.72% |
| rbench/awf/Mandelbrot | 265,025,513 | 277,492,022 | 276,402,745 | -0.39% | +4.29% |
| rbench/awf/Storage | 655,694,651 | 668,778,449 | 666,850,168 | -0.29% | +1.70% |
| rbench/real/convolution | 371,382,909 | 384,761,800 | 383,771,226 | -0.26% | +3.34% |
| rbench/real/convolution_slow | 2,108,717,880 | 2,121,117,964 | 2,128,120,612 | +0.33% | +0.92% |
| rbench/real/convolution_v | 698,544,186 | 711,809,775 | 710,824,038 | -0.14% | +1.76% |
| rbench/real/flexclust | 4,783,611,542 | 4,773,819,325 | 4,772,765,039 | -0.02% | -0.23% |
| rbench/real/flexclust_no_s4 | 5,234,614,123 | 5,220,910,641 | 5,217,340,700 | -0.07% | -0.33% |
| rbench/real/volcano | 255,169,355 | 268,562,133 | 267,543,935 | -0.38% | +4.85% |
| rbench/shootout/binarytrees | 470,126,063 | 479,481,038 | 478,009,061 | -0.31% | +1.68% |
| rbench/shootout/binarytrees_2 | 465,986,559 | 477,254,560 | 475,659,567 | -0.33% | +2.08% |
| rbench/shootout/binarytrees_naive | 464,620,812 | 475,958,731 | 474,379,809 | -0.33% | +2.10% |
| rbench/shootout/fannkuchredux | 255,023,197 | 268,407,169 | 267,366,450 | -0.39% | +4.84% |
| rbench/shootout/fannkuchredux_naive | 255,032,745 | 268,427,732 | 267,462,736 | -0.36% | +4.87% |
| rbench/shootout/fasta | 1,047,931,749 | 1,062,334,715 | 1,059,623,082 | -0.26% | +1.12% |
| rbench/shootout/fasta_2 | 1,175,826,566 | 1,191,512,351 | 1,188,380,381 | -0.26% | +1.07% |
| rbench/shootout/fasta_3 | 1,033,636,194 | 1,048,873,852 | 1,045,996,173 | -0.27% | +1.20% |
| rbench/shootout/fasta_naive | 1,022,011,476 | 920,870,300 | 917,409,974 | -0.38% | -10.23% |
| rbench/shootout/fasta_naive_2 | 802,546,254 | 817,687,239 | 814,944,323 | -0.34% | +1.54% |
| rbench/shootout/fastaredux | 1,153,558,066 | 1,167,048,286 | 1,163,371,049 | -0.32% | +0.85% |
| rbench/shootout/fastaredux_naive | 1,082,759,344 | 1,106,256,585 | 1,102,649,595 | -0.33% | +1.84% |
| rbench/shootout/knucleotide | 255,599,337 | 268,928,753 | 267,902,622 | -0.38% | +4.81% |
| rbench/shootout/knucleotide_brute | 255,755,637 | 269,129,594 | 268,079,007 | -0.39% | +4.82% |
| rbench/shootout/knucleotide_brute_2 | 255,541,820 | 268,919,877 | 267,887,355 | -0.38% | +4.83% |
| rbench/shootout/knucleotide_brute_3 | 255,524,143 | 268,945,593 | 267,964,953 | -0.36% | +4.87% |
| rbench/shootout/mandelbrot_ascii | 756,251,838 | 1,039,093,288 | 540,566,866 | -47.98% | -28.52% |
| rbench/shootout/mandelbrot_naive_ascii | 1,356,693,720 | 1,370,246,113 | 1,307,164,774 | -4.60% | -3.65% |
| rbench/shootout/mandelbrot_noout | 950,008,329 | 1,366,769,508 | 628,572,230 | -54.01% | -33.84% |
| rbench/shootout/mandelbrot_noout_naive | 845,114,422 | 976,451,180 | 735,144,333 | -24.71% | -13.01% |
| rbench/shootout/nbody | 831,913,778 | 847,316,620 | 843,167,494 | -0.49% | +1.35% |
| rbench/shootout/nbody_2 | 942,868,561 | 955,887,965 | 956,881,547 | +0.10% | +1.49% |
| rbench/shootout/nbody_3 | 1,047,943,892 | 1,060,389,752 | 1,061,615,766 | +0.12% | +1.30% |
| rbench/shootout/nbody_naive | 1,792,192,598 | 1,789,168,168 | 1,783,632,558 | -0.31% | -0.48% |
| rbench/shootout/nbody_naive_2 | 1,351,937,913 | 1,364,602,362 | 1,367,373,888 | +0.20% | +1.14% |
| rbench/shootout/pidigits | 452,993,996 | 465,718,888 | 463,814,902 | -0.41% | +2.39% |
| rbench/shootout/regexdna | 255,222,387 | 268,562,517 | 267,543,680 | -0.38% | +4.83% |
| rbench/shootout/reversecomplement | 255,380,109 | 268,698,983 | 267,687,315 | -0.38% | +4.82% |
| rbench/shootout/reversecomplement_2 | 255,420,156 | 268,687,433 | 267,642,331 | -0.39% | +4.79% |
| rbench/shootout/reversecomplement_naive | 255,434,234 | 268,788,828 | 267,623,759 | -0.43% | +4.77% |
| rbench/shootout/spectralnorm | 771,912,753 | 785,725,756 | 784,145,679 | -0.20% | +1.58% |
| rbench/shootout/spectralnorm_alt | 2,276,967,796 | 1,012,447,925 | 1,011,369,324 | -0.11% | -55.58% |
| rbench/shootout/spectralnorm_alt_2 | 968,160,185 | 859,415,237 | 858,272,317 | -0.13% | -11.35% |
| rbench/shootout/spectralnorm_alt_3 | 2,188,821,989 | 2,198,272,213 | 2,187,357,179 | -0.50% | -0.07% |
| rbench/shootout/spectralnorm_math | 341,531,462 | 354,997,439 | 353,361,094 | -0.46% | +3.46% |
| rbench/shootout/spectralnorm_naive | 1,045,410,467 | 1,058,366,406 | 1,056,286,920 | -0.20% | +1.04% |
| rbench/simple/bytecodes | 272,218,953 | 285,548,342 | 284,536,347 | -0.35% | +4.52% |
| rbench/simple/emptyFor | 340,755,061 | 354,179,866 | 353,160,745 | -0.29% | +3.64% |
| rbench/simple/emptyWhile | 290,399,827 | 303,747,659 | 302,713,575 | -0.34% | +4.24% |
| rbench/simple/lapply | 351,812,141 | 365,131,527 | 364,198,256 | -0.26% | +3.52% |
| rbench/simple/lapplyDots | 763,840,207 | 765,990,551 | 763,482,792 | -0.33% | -0.05% |
| rbench/simple/listFor | 346,490,079 | 359,868,592 | 359,147,119 | -0.20% | +3.65% |
| rbench/simple/listWhile | 458,738,562 | 472,081,603 | 471,880,721 | -0.04% | +2.86% |
| rbench/simple/matrixFor | 752,811,572 | 766,202,929 | 766,950,458 | +0.10% | +1.88% |
| rbench/simple/profiler-microbenchmark | 780,497,741 | 787,443,711 | 785,363,318 | -0.26% | +0.62% |
| rbench/simple/profiler-rsa | 537,835,876 | 551,033,222 | 549,000,592 | -0.37% | +2.08% |
| rbench/simple/profiler-shared | 6,729,961,721 | 6,425,484,241 | 6,364,281,996 | -0.95% | -5.43% |
| rbench/simple/scalarFor | 276,547,471 | 289,920,455 | 288,881,312 | -0.36% | +4.46% |
| rbench/simple/scalarWhile | 279,437,974 | 292,835,575 | 291,829,819 | -0.34% | +4.43% |
| rbench/simple/superWhile | 1,314,103,002 | 1,326,699,714 | 1,314,272,374 | -0.94% | +0.01% |
| rbench/simple/vectorFor | 1,472,539,066 | 1,485,936,269 | 1,491,136,253 | +0.35% | +1.26% |
| rbench/simple/vectorWhile | 1,625,701,029 | 1,639,125,971 | 1,644,710,418 | +0.34% | +1.17% |
| robigo/apply | 896,780,117 | 910,886,228 | 908,026,497 | -0.31% | +1.25% |
| robigo/binarytrees | 1,467,117,508 | 1,398,399,057 | 1,398,521,012 | +0.01% | -4.68% |
| robigo/csv | 935,115,777 | 941,877,565 | 940,564,435 | -0.14% | +0.58% |
| robigo/dataframe | 705,317,235 | 1,272,642,771 | 666,763,005 | -47.61% | -5.47% |
| robigo/fannkuch | 1,289,210,621 | 1,302,421,362 | 1,306,188,222 | +0.29% | +1.32% |
| robigo/fib | 1,190,115,178 | 1,111,331,219 | 1,110,615,818 | -0.06% | -6.68% |
| robigo/lm | 733,677,197 | 743,301,420 | 742,078,927 | -0.16% | +1.15% |
| robigo/mandelbrot | 305,277,621 | 318,711,972 | 317,686,875 | -0.32% | +4.06% |
| robigo/matrix | 725,939,173 | 740,062,312 | 739,095,043 | -0.13% | +1.81% |
| robigo/nbody | 452,501,627 | 465,702,366 | 464,672,228 | -0.22% | +2.69% |
| robigo/queens | 381,785,376 | 391,050,841 | 389,744,585 | -0.33% | +2.08% |
| robigo/spectralnorm | 323,200,694 | 336,485,497 | 335,507,388 | -0.29% | +3.81% |
| robigo/strings | 846,618,338 | 859,339,534 | 857,223,588 | -0.25% | +1.25% |
| robigo/vecarith | 4,433,938,385 | 4,462,460,831 | 4,461,415,027 | -0.02% | +0.62% |
| robigo/vecops | 2,508,782,900 | 2,522,192,362 | 2,521,163,899 | -0.04% | +0.49% |

geomean this PR / main: 0.9673

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
  10,988 cases) in the container: main 963b3cf 10,638 -> 10,648, none lost.
  The 10 gained are NaN-payload cases of complex `+`, whose `re` part is a
  plain Rust `+` and depends on LLVM's operand order; the new kernel
  happens to emit GNU R's order. The ratchet's 15-20 "regressions" in this
  slice (complex `*`/`%*%` NaN payloads, e.g. `(1i+NA)*(0/0)`, and three
  others) fail the same way with CI's own binary of main and are not
  failures in CI's corpus job.
- Benchmark output against GNU R (`perf.py run --metric time --runtime
  both`): the same 3 wrong programs as main (`fannkuchredux`,
  `fannkuchredux_naive`, `spectralnorm_math`).
