# No collection in a row's window; what the bars' ratios hold of GNU R's collector (2026-11-23)

**30 of 63 rows collect inside the N-to-2N window on GNU R's side, and
almost none do on robigo-rt's. So today's ratios compare GNU R's mutator
plus its minor collections against robigo-rt's mutator alone.** robigo-rt's
collections are not free, only rarer and outside the window: a session pays
them later, and each one traces the whole live heap until the boot freeze
lands. So the bars flatter robigo-rt by GNU R's collector share of each
allocating row: s3 is 0.873× today and 1.060× when neither side collects
(1.104× with both sides' collections amortised). The table below has three
numbers per row for the decision. This entry changes no bar.

X/P1. It follows the foreman's class D finding: guard-vec-arith was +1.20%
under the rdb boot against the source boot on main.

## Class D: one collection inside the window

guard-vec-arith (`y <- x * 2 + x` on 1,000 doubles: two 8 KB vectors per
iteration) collected 0 times at N = 4000 and once at 2N, under every boot.
The row's figure, (2N run − N run) / N, therefore held one whole collection.
A collection's cost follows the live heap, and the rdb boot leaves more:

| boot | Ncells | Vcells | namespaces |
|---|---:|---:|---|
| source (interim) | 9,076 | 6,939 | base |
| rdb, JIT on | 47,758 | 99,869 | compiler, base |
| rdb, JIT off | 24,836 | 44,170 | base |

The diff by function, per iteration, main 0007ddf33, shift 0:
- the collector: `gc::Core::trace` +724 and `finish_collection` +329,
  4.24M instructions for the one collection;
- the loop itself: the same, 16,125.7 against 16,128.9 in `real_binary`'s
  closure.

With the heap sized so that neither run collects (`R_VSIZE=256M`), the row
reads 87,611 under the source boot and 87,612 under rdb. The figure drops
by 575 per iteration under the source boot (88,186 → 87,611) and 1,634
under rdb (89,246 → 87,612), because the collection left the window.

**ROBIGO_GC_FREEZE=1** (the boot heap's freeze, #959's first step) made the
row worse: +17.7% under rdb. The freezing collection falls inside the window
too, and under rdb it thaws much of what it froze: `thaw_with_referrers`
≈ 19M instructions, plus the sort and `has_ordinary_child`. H's.

**A first reading of "rdb with the JIT off is +1.55% per iteration" was
wrong.** It compared JIT on against JIT off, not the boots: the source boot
compiles at JIT level 3 by default. With the JIT off under both boots, both
show the same four lookups per iteration (`{`, `<-`, `+`, `*`, as GNU R's
AST interpreter does), and the remaining +0.55% is the collection again.

## The check (idiom-vs-r.py --rows)

- **Counts before measuring:** every row runs once at N and once at 2N
  without callgrind.
  - robigo-rt counts its collections: the heap's `ROBIGO_GC_STATS=1` report.
  - GNU R counts its own: `gcinfo(TRUE)`'s lines, in a run of its own, so
    the measured script stays as it is.
- **robigo-rt's side is enforced:** its two counts must be equal and zero,
  unless the row is marked `collects = true`. Otherwise the row is refused
  before anything is measured, with the row, the counts and the boot. All
  refused rows are named at once. A heap shift's run (`--shifts`) is
  checked again, since its strings can move the collector's cycle into the
  window.
- **GNU R's side is reported, not enforced:** its generational collector
  makes minor collections inside most allocating rows' windows, and
  enforcing that is the decision below. Both sides' counts are in the
  table and the JSON.
- **`heap = { R_VSIZE = "...", R_NSIZE = "..." }`** sizes the heap of both
  sides' runs: GNU R reads the same variables (startup.c:R_SizeFromEnv).
  guard-vec-arith gets `R_VSIZE = "256M"`: 0 → 0 on robigo-rt under all
  three boots, and on GNU R 4.6.2 too.
- **The jit rows are `collects = true`:** a compile allocates.
  `perf.py rt-bars` prints their collection counts for both binaries and
  warns, without failing, when the PR's differ from main's. Then the row's
  change holds a collection's cost, not only compile work.

guard-vec-arith's ratio, re-taken under the same sizing on both sides
(GNU R 4.6.2, the rt pin; main 0007ddf33, source boot):

| | robigo-rt | GNU R | ratio | collections, N→2N |
|---|---:|---:|---:|---|
| default heap | 88,188 | 18,085 | 4.88× | 0→1 on both sides |
| `R_VSIZE=256M` | 87,611 | 14,993 | 5.84× | 0→0 on both sides |

GNU R's collection left the window too, and its share was larger: −3,092
per iteration against robigo-rt's −577.

## The census

Each row of calls-rows.toml at N and 2N, main 0007ddf33.
- **robigo-rt**, under the source boot, rdb, and rdb with the JIT off:
  every row but source5k collects 0 → 0, except the ones below. source5k
  is not a measured row on robigo-rt: its script stops at an error under
  all three boots (`Error: package ‘utils’ does not have a namespace`:
  base's `source()` calls `utils::localeToCharset()`, and robigo-rt's
  library has no utils; checked on main 5d961809e). The census read the
  heap report, which a failed run still prints as 0 collections, without
  checking the exit; `--rows` now refuses such a row by name. The table
  below does not include source5k.
  - guard-vec-arith: 0 → 1 under all three;
  - fib: 0 → 1 under rdb with the JIT off only;
  - the seven jit rows: 2–8 → 4–16. Under rdb, four of them do one more
    collection at 2N than under the source boot (scale_by, weigh, tag,
    safe).
- **GNU R 4.6.2:** 30 of 63 rows collect inside the window:
  - all seven jit rows, 2N ≈ 2 × N;
  - s3, trycatch-ok, trycatch-err, try-err;
  - fib 3 → 5, binarytrees 1 → 2, lapply 1 → 2, source5k 14 → 30;
  - d2s-assign, d2s-eval, d2s-callee-assign, henv, with, l2e, nenv;
  - grow-dbl, grow-list, runonce;
  - d0-cbind, d0-rbind, d0-paste, d0-paste0;
  - guard-vec-arith.

## Three ratios per row (for the decision; no bar changes here)

robigo-rt (main 0007ddf33, source boot) against GNU R 4.6.2, callgrind
instructions per iteration, both on this machine, heap shift 0. The
columns:
- **today:** the default heaps, as the bars measure.
- **zero:** both sides at `R_VSIZE=4G R_NSIZE=40000000`, neither collecting
  in the window (checked: 0 → 0 on both sides for every row): the November
  "mutator-only" proposal.
- **amortised:** each side's N doubled until its window held ≥ 10
  collections, or until the side's native 2N run passed 2 s, whose count
  the column states. For a side that never collects at any N tried (its
  loop allocates nothing that triggers one), its zero figure.

| row | bar | today | zero | amortised | robigo-rt's collections in its amortised window | GNU R's today, N→2N |
|---|---|---:|---:|---:|---|---|
| plain | yes | 0.424 | 0.424 | 0.428 | never | 0→0 |
| defaults | yes | 0.351 | 0.351 | 0.355 | never | 0→0 |
| named | yes | 0.420 | 0.420 | 0.425 | never | 0→0 |
| env-fn | yes | 0.535 | 0.535 | 0.539 | never | 0→0 |
| superassign | yes | 0.609 | 0.609 | 0.620 | 6 at N = 2,048,000 | 0→0 |
| s3 | yes | 0.873 | 1.060 | 1.104 | 15 at N = 512,000 | 0→1 |
| fib | yes | 0.471 | 0.464 | 0.468 | never | 3→5 |
| binarytrees | yes | 0.977 | 0.984 | 1.010 | 8 at N = 2,560 | 1→2 |
| e7-paste | yes | 0.538 | 0.538 | 0.505 | 8 at N = 25,600 | 0→0 |
| e7-match | yes | 0.480 | 0.480 | 0.481 | 7 at N = 51,200 | 0→0 |
| e7-unique | yes | 0.778 | 0.778 | 0.779 | 10 at N = 51,200 | 0→0 |
| jit-scale_by | yes | 1.384 | 1.325 | 1.384 | 4 at N = 400 | 21→42 |
| jit-weigh | yes | 1.395 | 1.331 | 1.395 | 4 at N = 400 | 19→39 |
| jit-tag | yes | 1.395 | 1.331 | 1.395 | 5 at N = 400 | 20→41 |
| jit-env-fn | yes | 1.379 | 1.285 | 1.379 | 3 at N = 400 | 11→21 |
| jit-area | yes | 1.336 | 1.268 | 1.330 | 4 at N = 800 | 9→18 |
| jit-area-sq | yes | 1.370 | 1.300 | 1.360 | 5 at N = 800 | 13→26 |
| jit-safe | yes | 1.361 | 1.307 | 1.361 | 6 at N = 400 | 32→64 |
| trycatch | | 1.223 | 1.221 | 1.272 | 6 at N = 32,000 | 1→1 |
| trycatch-ok | | 0.879 | 1.161 | 1.197 | 4 at N = 64,000 | 0→1 |
| trycatch-err | | 1.252 | 1.248 | 1.309 | 5 at N = 16,000 | 1→2 |
| trycatch-bare | | 0.522 | 0.522 | 0.526 | never | 0→0 |
| d2s-assign | | 0.980 | 1.227 | 1.256 | 6 at N = 640,000 | 0→1 |
| d2s-eval | | 1.817 | 2.170 | 2.223 | 9 at N = 320,000 | 0→1 |
| d2s-callee-assign | | 1.044 | 1.262 | 1.297 | 5 at N = 320,000 | 0→1 |
| d2s-delayed | | 1.465 | 1.461 | 1.500 | 4 at N = 160,000 | 1→1 |
| d0-attr | | 0.792 | 0.793 | 0.805 | never | 0→0 |
| d0-attributes | | 0.884 | 0.884 | 0.947 | 18 at N = 2,048,000 | 0→0 |
| d0-names | | 2.273 | 2.274 | 2.286 | never | 0→0 |
| d0-unlist | | 1.044 | 1.044 | 1.085 | 8 at N = 512,000 | 0→0 |
| d0-cbind | | 1.115 | 1.505 | 1.622 | 8 at N = 128,000 | 0→1 |
| d0-rbind | | 1.101 | 1.483 | 1.593 | 15 at N = 256,000 | 0→1 |
| d0-paste | | 0.576 | 0.673 | 0.691 | 7 at N = 256,000 | 0→1 |
| d0-paste0 | | 0.560 | 0.663 | 0.675 | 7 at N = 256,000 | 0→1 |

What the columns show:
- **Rows where GNU R makes a minor collection in its window move most from
  today to zero:** s3, trycatch-ok, the d2s rows, d0-cbind/rbind and the
  d0 paste rows, by 15–40%. Today's figure holds GNU R's collection and
  not robigo-rt's.
- **Amortised is above zero for almost every allocating row.** Spread over
  many collections, robigo-rt's collector costs more per iteration than
  GNU R's minor ones: a full trace each time, against a generation's.
  That is the freeze's (H's) work. e7-paste is the exception (0.505).
- **The jit rows' amortised figure equals today's:** their N = 400 windows
  already hold many collections on both sides.
- **Per compile with neither side collecting:**
  - jit-scale_by 8,894,408 against GNU R's 6,712,560, 1.325× (today
    9,214,066 against 6,655,279, 1.384×);
  - jit-safe 13,331,107 against 10,199,204, 1.307× (today 13,812,964
    against 10,148,863, 1.361×).

  GNU R's own figure is slightly higher without collections (its minor
  collections cost it less than the extra pages a 4 GB heap leaves it
  allocating). robigo-rt's drops about 3.5%: its collections were part of
  the compile's cost.
- **Caveats:**
  - **The amortised robigo-rt side** holds 3–15 collections, not always 10:
    its 2N run hit the 2 s cap first. Its cost per collection follows the
    live heap, which a longer run grows little.
  - **e7-match is address-sensitive:** here only shift 0, where the bars
    use the median over shifts.
