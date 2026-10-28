# Ratchet integrity: why ratcheted failures did not fail CI

Status (2026-10-29): the enforcement (`defaults: run: shell: bash`) and
the fixes it needed landed with #62, found in parallel to #58 with the
same results (pipefail, parse-corpus's target directory, cran-purer's
`support/`, IGNORE_RDIFF blocks, the lexer's incomplete exponent,
`do.call`'s NA names, NA into complex vectors). #58 then carries only
what #62 lacks: the five machine-dependent cases scored again
(re-recorded with masks instead of skipped as `host_dependent`), the
GNU R complex oracle, and conformance scripts. The investigation below
is #58's.

## Problem

29 cases in the corpus ratchets (`tests/corpora/*/passing.txt`) failed on
`main` (CI run 40662552137, 414ae44: fastr 22, gnur-examples 7), yet every
corpus job passed.

## Cause: the gates' exit status was `tee`'s

`scripts/corpus.py run` exits with status 1 when a ratcheted case fails
(`return 1 if regressions else 0`), but CI runs it as

    python3 scripts/corpus.py run ... | tee -a "$GITHUB_STEP_SUMMARY"

and, in a container job with no `shell:` given, GitHub runs steps with
`sh -e {0}`: no `pipefail`, so a pipeline's status is its last command's,
`tee`'s, always 0. The step logs confirm it: "shell: sh -e {0}", then
"fastr: ...; 144 new, 4 regressions", step conclusion "success". The same
pattern disables the other two gates: the parse-corpus ratchet
(`parse-corpus.py check ... | tee`) and the perf gate
(`perf.py compare ... | tee`, so the 0.5% rule has never failed a run
either; #9 and #29 merging with regressions was not only stale CI).
Sharding, tolerance and normalization are not involved: each shard
compares its own cases with the ratchet and reports the regressions
correctly.

## Fix (enforcement)

Applied in `.github/workflows/ci.yml` (59e06f9, once the `workflow`
token scope was granted): a workflow-level `defaults: run: shell: bash`,
which GitHub runs as `bash --noprofile --norc -eo pipefail {0}`. This
PR's own CI is the first run with enforced gates.

Every other pipeline in the workflow and the setup action was checked:
`cargo metadata | python3` and `echo | sed` (ci.yml) succeed under
pipefail; the setup action's steps already declared `shell: bash` (so
pipefail: `R --version | head -1`, `curl | sh`); `[ ... ] && x=...` lines
are exempt from `-e`; the result artifacts are uploaded with
`if: always()`.

### What enforcement exposed

- The first enforced run (63164f3) failed the build job in the
  parse-corpus ratchet step: `parse-batch not found at
  /github/home/.cache/cargo-target/release/parse-batch`.
  `scripts/parse-corpus.py` guessed the target directory as
  `$CARGO_TARGET_DIR` or `~/.cache/cargo-target` (this machine's
  `~/.cargo/config.toml`); CI builds into `./target`. So the parse
  ratchet has never run in CI. It now asks `cargo metadata` for the
  target directory, as the workflow's binary-copy step does. This run
  also confirms that a failing gate fails the job: the step ran under
  `bash ... -o pipefail`, the script exited 1 behind `| tee`, the job
  failed.

## The 29 cases

On main 45b4dd2, 22 of them had been fixed meanwhile by other tracks;
this branch fixes the other 7 and adds tests for all.

| cases | cause | fix |
|---|---|---|
| 10 TestSimpleValues `*` (e.g. `(1i+NA)*(0/0)`), 2 TestSimpleArithmetic `%*%` | NaN payloads (NA vs NaN) of complex products: GNU R's compiled `x1 * x2` is `__muldc3(x2, x1)`, and the `__muldc3` linked into libR.so computes `c * b` | fixed on main by #47 (a28b980) and #43; independently confirmed here (below) and pinned by a GNU R oracle test |
| 5 TestSimpleValues `+` (e.g. `(0/0)+(1i+NA)`) | complex `+` was a plain Rust `+` (LLVM picks the operand order); GNU R's `x1.r + x2.r` has `x2` as the destination | fixed on main by #47 (`cadd`); pinned by the oracle |
| TestBuiltin_rapply, TestBuiltin_Reduce; gnur-examples base/rapply, base/funprog | `log` (a special) evaluated its argument expressions one by one: `f(...)`/`log(...)` raised "'...' used in an incorrect context" | fixed on main (c867234, `evalListKeepMissing`); `tests/r/math/log-args.R` covers these shapes |
| TestS3Dispatch/0cd27030 | `log(x)` on a classed object dispatched `Math` with `x` and `base` | fixed on main (R's `n == 1` branch); covered by log-args.R |
| **TestBuiltin_operators/0bb20eaa** | a 4,951-byte line: the console reads 4,096 bytes at a time (as R does) and the cut fell after `1.79193891693885e-`; robigo's lexer reported "unexpected input", R's `R_Parse1` reports any error after the lexer read past the buffer as PARSE_INCOMPLETE | **this branch**: the same rule for lexical errors (robigo-syntax lexer); `tests/r/basics/long-line-exponent.R` |
| **TestBuiltin_order/add62673; gnur-examples base/Sys.info, base/Sys.which, base/find.package, parallel/detectCores** | recorded on a machine that is not the oracle platform (it had `en_US.utf8`, `ping`, MASS, 2 cores, host name `v2202…`); CI's Debian trixie has none of these, so GNU R itself would not reproduce the recording there | **this branch**: re-recorded on the oracle platform (`robigo-oracle:4.5.0` = CI's setup) with `record --mask-against-previous`: lines that differ between the two machines are masked (Sys.info's block, 1 line each for Sys.which/find.package, the two detectCores lines) |
| **gnur-examples base/round.POSIXt** | `trunc(Sys.time(), "day")`, the date of the recording, inside `## IGNORE_RDIFF_BEGIN/END` | **this branch**: the comparison drops IGNORE_RDIFF blocks like GNU R's own `tools::Rdiff` (after line masking, so recorded masks keep their indices) |

Two more ratcheted gnur-examples cases broke on main after 414ae44 (CI
run of 45b4dd2: 7 gnur-examples regressions, the 5 above plus these);
fixed here too so that enforcing the gates does not turn CI red:

| case | cause | fix |
|---|---|---|
| base/colSums | `x[3, ] <- NA` on a complex matrix stored `NA + 0i`; R's subassign stores `NA_complex_` (subassign.c cases 1510/1513/1514: NA in both parts), and `colMeans(na.rm = TRUE)` averages Re and Im separately | `assign_values` converts logical/integer/double values into complex as subassign does; `tests/r/vec/complex-na-assign.R` |
| base/as.data.frame | `do.call` dropped an argument name that is `NA_character_`; R tags it with the symbol `NA` (`ItemName` is "no name" only for `""`), so `names(as.data.frame(L))` with `names(L) <- NA` is `"NA."` | `do_docall` keeps NA names as `NA`; `tests/r/lang/docall-na-names.R` |

No ratchet entry was removed.

### Complex arithmetic as GNU R's binary computes it

Found here in parallel with #47 (which landed first with the same
conclusions); kept as the record of the evidence. From `objdump -d` of Debian's `libR.so` (r-base-core 4.5.0-3, gcc 14;
`__muldc3`/`__divdc3` are linked statically from libgcc.a) and GNU R
itself:

- `complex_binary`'s `TIMESOP` loop computes the product inline with the
  second operand `x2` as the destination of every `mulsd`, and calls
  `__muldc3(x2.r, x2.i, x1.r, x1.i)` when either part is NaN (`ucomisd`
  parity). Without NaNs the order cannot change the result, so
  `x1 * x2` = `__muldc3(x2, x1)`.
- That `__muldc3` computes `ac = a*c`, `bd = b*d`, `ad = a*d`, but
  `bc = c*b` (destination `c`); the infinity recovery multiplies by
  `INFINITY` with the other operand as destination, which cannot change a
  payload.
- `PLUSOP`: `x1.r + x2.r` is `addsd` with `x2` as the destination.
  `MINUSOP` (necessarily `x1` first) and `__divdc3` already matched.
- `%*%` with NaN/Inf operands (`simple_cmatprod`): `sum_r = 0;
  sum_r += creal(xij * yjk)` with the product compiled as above.
- x86-64 SSE: a NaN operand is returned quieted, the destination's
  payload winning when both are NaN, including signalling NaNs such as
  `NA_real_` (`0x7ff00000000007a2`); checked with a `mulsd` probe in the
  container.

An oracle of GNU R's own results, `complex_r_oracle.txt.gz` (from
`scripts/oracle-complex-arith.R`): every pair of the 196 complex numbers
whose parts are signalling and quiet NA, NaN with four payloads, ±Inf, ±0,
1, -3, the largest double and the smallest subnormal, for `+ - * / ^` and
`%*%` (230,496 results), now the unit test `gnu_r_oracle`. main 414ae44
(CI binary): `*` 9,828, `+` 10,192, `%*%` 9,828 mismatches; main 45b4dd2
(after #43, #47): 0. A second implementation of the same rules on this
branch before #47 also reached 0 and was dropped for main's.

## Verification (head on main 45b4dd2)

- The 29 cases with the branch's binary, in the container: fastr 22/22,
  gnur-examples 7/7. With main 45b4dd2's binary and main's harness and
  recordings: fastr 20/22 (TestBuiltin_operators/0bb20eaa and
  TestBuiltin_order/add62673 fail), gnur-examples 2/7 (Sys.info,
  Sys.which, find.package, round.POSIXt, detectCores fail).
- Full corpora against their ratchets with this branch's harness, in the
  container (on b6fc7c8 + these changes): fastr 30,305/31,374,
  gnur-examples 557/1,153, gnur-tests 20/59, gnur-demos 2/16, r-intro
  4/13, cran-purer 31/41, capi 14/20: **0 regressions in every corpus**,
  so enforcing the gates does not turn CI red. (Not re-run in
  full on 45b4dd2 with the machine's memory constrained; CI's corpus jobs
  on this PR show the counts.)
- cran-purer from a fresh clone of the branch: the build installs the
  pinned packages and the run gives 31/41, 0 regressions (main: the build
  fails, see the table above; CI uploaded no cran-purer results).
- Parse ratchet (`parse-corpus.py check fastr gnur-tests gnur-examples`):
  all match, +0/-0.
- `cargo fmt --check`, `cargo test --release --workspace` (conformance in
  all tiers; unit tests `gnu_r_oracle`, the C library oracle) in the
  container (memory-capped, 2 build jobs). New conformance scripts
  `tests/r/math/log-args.R`, `tests/r/math/complex-nan-payloads.R`,
  `tests/r/basics/long-line-exponent.R`, ratcheted.
- gnur-examples in full and a fastr slice (subassign, do.call, complex,
  data frames, the TestSimple* families; 12,237 cases) with the
  subassign/do.call fixes: gnur-examples 619/1,153 and fastr
  12,201/12,237, 0 regressions in both.
- Perf (callgrind, container, `--jobs 2`, memory-capped): a **targeted**
  run of the programs the changes can touch (complex arithmetic:
  mandelbrot* x5; `%*%`: robigo/matrix; reading and parsing:
  knucleotide, regexdna, reversecomplement, pidigits, robigo/strings;
  `[<-`: matrixFor, vectorFor, vecops; `do.call`: dataframe, lm, csv,
  apply) and controls (fib, shootout/nbody, spectralnorm, vecarith,
  binarytrees_naive), 23 programs, against main 45b4dd2: 0 faster, 0
  slower than 0.5%; all within -0.05%..+0.07%.
- CI's perf job reported binarytrees_naive +0.61% against main 45b4dd2 on
  both pushes (every other program within 0.09%). CI's own two binaries
  (main 45b4dd2's and this PR's artifact), run here under callgrind:
  492,583,019 vs 492,581,893 instructions, the same. The difference is
  the runner, not the code: what moves is mimalloc's purging
  (`_mi_arenas_collect`, `mi_arena_try_purge_visitor`), which runs on a
  time delay, so allocation-heavy programs' counts depend on wall-clock
  speed (here the same binary gives 492,476,820 / 492,547,183 /
  492,551,029 in three runs). Once the gates fail for real this makes the
  perf gate flaky; follow-up, not done here: run the benchmarks with
  mimalloc's time-based purging off (e.g. `MIMALLOC_PURGE_DELAY=-1` in
  perf.py's environment; untested) and re-baseline.
- Observed, not changed here: main's mandelbrot_noout is 690.8M at 45b4dd2
  against 648.4M at d8a097f (+6.5%), not bisected. A likely cause is
  #47's `cadd`, which computes a plain sum and recomputes it exactly when
  it is NaN, as the escaping points' sums are; an exact packed `addpd`
  (tried on this branch before #47: 645.1M) would avoid the second pass.
