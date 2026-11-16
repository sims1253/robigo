## 2026-11-16: M1 exit evidence: E1–E9 measured on main `c536597d`

The evidence index for the council's M1 exit review (`docs/g3.md` §15.3).
For each criterion it gives the bar as §15.3 words it, its status, the
measurement on the final main, earlier entries where they differ, what
proves it in CI, and the gaps. Then come the decisions M1 rests on, the
items M1 leaves open with their owners, and questions for the council.
No code changed.

> **Final state.** Re-measured on main `c536597d`, after S1's ALIAS
> (#731), one heavy run at a time. The first run (main `2671f453`) is
> kept where it differs, marked "first run". This state folds in the
> revisions since then:
> - GNU R's baseline is the oracle image's, env-fn is an M1 row at
>   ≤ 1.0x, and E6's hook cost is measured
>   (`2026-11-16-bars-oracle-baseline.md`).
> - The `perf-rt` job gates the instruction and peak-RSS bars
>   (`2026-11-16-m1-bars-gate.md`).
> - The lead's decisions on E2's fib (`2026-11-16-e2-fib-decision.md`)
>   and on E7's paste ceiling (`2026-11-15-e7-seq.md`).
> - Four of the first run's five failing cases now pass (#730).
>
> New here: the decisions, the open items with owners (the stop-gaps
> re-checked on `c536597d`) and the questions for the council.
>
> After the council's first round:
> - E1, E2 and E7's headline numbers are CI's `perf-rt`, which the gate
>   enforces. The hand run on this host is kept as a second column,
>   labelled host-vs-image.
> - E1's wall time is measured under §13's rule (non-PGO; robigo-rt has
>   no PGO build).
> - E5's mode accounting names both excluded cases, and says which crate
>   tests run under each mode.
> - `docs/g3.md` §16's sentence on CI jobs is corrected.
> - E8 states that its expectations are hand-written Rust assertions,
>   run with inlining on only. E9 separates what CI enforces per PR from
>   T1's one-off sweep.
> - The lead's three decisions of 2026-11-16 are recorded
>   (`2026-11-16-m1-exit-lead-decisions.md`): a non-PGO binary for E1's
>   wall time in M1, GNU R-recorded cases for E8 before sign-off, and a
>   study of estimated cycles.
>
> After the council's second round:
> - C1c, the link-count fix, is listed with E8's cases as blocking M1's
>   sign-off.
> - E8 reads "met in Rust".
> - The wall-time and paste figures and the open item on §13's rule are
>   corrected.

- **Tree:** main `c536597d` (#731). Every number below was measured on
  this tree. Exceptions are marked: GNU R 4.6.2's counts, today's
  robigo's (G1), and older entries cited for what they measured.
- **Binary:** `cargo build --release --locked -p robigo-rtapi`
  (`robigo-rt`), sha256 `7de68411996a8821…`.
- **Machine:**
  - **The hand run:** AMD Ryzen 9 5900X (12 cores), 47 GiB, WSL2 Linux
    6.18, glibc 2.44, rustc 1.100.0, valgrind 3.25.1, the first run's.
  - **`perf-rt`:** GitHub's `ubuntu-24.04` runner, with robigo-rt built and
    measured in CI's Rust image, Debian trixie (glibc 2.41). The job does
    not print the runner's CPU (open items).
- **GNU R 4.6.2:** the oracle image `robigo-oracle:4.6.2` (Debian glibc
  2.41), committed in `benchmarks/rt/m1-bars.toml` from
  `2026-11-16-bars-oracle-baseline.md`. It was not re-run: that entry's
  two runs agree within 0.0001%.
  - **The local build it replaced**
    (`~/.cache/robigo-research/R-4.6.2-build`, the baselines' provisional
    binary, `../g3.md:419-421`): the oracle image counts 3.9% to 12.9%
    more instructions than it does. Ratios against it are given where
    they change a verdict.
- **Commands,** in this order, one at a time:
  - `scripts/rt-bars.sh` twice, with `--rows trycatch`;
  - the e7 rows: `scripts/idiom-vs-r.py --rows
    benchmarks/rt/calls-rows.toml --filter '^(e7-paste|e7-match|e7-unique)$'
    --cycles --no-r`, twice;
  - `scripts/rss.py --runs 5 --no-r`;
  - `scripts/rivals.py run --filter 'calls$' --impl
    robigo,gnur,rir,fastr --runs 5`;
  - `scripts/rt-stress.sh --runs 30`;
  - `scripts/rt-ci.py`, every step;
  - the E6 probe, `crates/robigo-rt/tests/hook_cost.rs`, under callgrind,
    N = 5,000 against 10,000, twice;
  - E1's wall time under §13's rule: `data/…/wall13.py`, 11 interleaved
    rounds.

  CI: `perf-rt` on main `c536597d` (run 43918105805, job 133477890224):
  `scripts/perf.py rt-bars`, the bars' rows in instructions, and
  `rss.py --runs 3`.

  Every two runs agree within 0.006% in instructions and 0.01% in
  estimated cycles.
- **Raw data:** `data/2026-11-16-m1-exit-c536597d/`. The first run's is
  `data/2026-11-16-m1-exit-2671f453/`.

### Summary

| | criterion | status | on `c536597d` |
|---|---|---|---|
| E1 | calls.R rows | met | `perf-rt`: the six M1 rows 0.33x-0.99x GNU R, s3 0.991x (met, 0.9% margin); wall time under §13, medians of 11, non-PGO: 0.321 s against Ř 1.051 s and FastR 9.159 s |
| E2 | fib, binarytrees | not met: fib, by the lead's decision | `perf-rt`: fib 1.272x today's robigo (bar 1.0x), owed to G8; binarytrees 0.840x, peak RSS 23,704 KiB (host 26,472) against 32,156 and 42,060 |
| E3 | determinism | met | stress 30 of 30 per profile; weak-keys passes; gctorture and the moving collector clean |
| E4 | overshoot, peak RSS | met on the rt slice | 628 runs, none over the bound, largest 0.238 of it; peak RSS 0.55x-0.84x GNU R |
| E5 | torture identity | met | both modes match production, but for the cases the modes exclude; `getBindingType` matrix passes |
| E6 | the API | met | modules, reviews, capi cases and regression cases done; the hook table costs 1.8 instructions per `lapply` element and nothing measurable on `[` |
| E7 | strings | met (accepted regression) | `perf-rt`: match 0.866x, unique 0.699x; paste 1.930x, accepted by the lead up to 2.0x |
| E8 | unwinding | met in Rust; GNU R-recorded cases pending, blocking sign-off | the three cases pass in Rust, in both profiles and across a C frame |
| E9 | allocation failure | met on the rt slice | conformance under `R_MAX_VSIZE=16M` 294/295; 400 injected failures, no difference |

One rt case fails, the same in every mode, with no marker:
`args/binding-types.R`. It needs regex (open items, below). The first run's
other four failures (`env/binding-errors`, `env/closure-frames`,
`env/hashed-env`, `env/unhashed-env`) pass and are ratcheted since #730.

### What CI proves, and what only hand runs prove

- **The `rt` job** runs on every pull request except docs-only ones. Its
  steps are `scripts/rt-ci.py`'s. It covers:
  - E3, with 3 stress runs per profile, not 30;
  - E4's overshoot;
  - E5, E8's Rust tests and E9.
- **The `perf-rt` job** (`2026-11-16-m1-bars-gate.md`) runs beside `rt`
  and `Check` needs it. It measures robigo-rt's rows in callgrind
  instructions, and peak RSS, against the committed GNU R 4.6.2 and
  today's-robigo numbers and against main's binary of the merge base, on
  the same runner. It fails a PR that:
  - breaks a met bar of E1, E2 or E7, or a peak-RSS bar;
  - slows any row by more than 0.5% (peak RSS: 2%);
  - takes e7-paste above its accepted ceiling.
- **Hand runs only:**
  - E1's wall time;
  - E2's estimated cycles;
  - E3's 30-run count;
  - E6's hook cost.

### E1: `calls.R`

> `calls.R` (one process, JIT off), M1 rows: plain, defaults, named,
> `e$f`, `<<-`, S3 ≤ 1.0x GNU R's instructions, the plain-call idioms
> ≤ 0.6x; wall time below Ř's and FastR's one-process. M3 adds `tryCatch`.

**Met.** s3 is under its bar since S1's ALIAS (#731): met, with a 0.9%
margin.

The enforced numbers are `perf-rt`'s on main `c536597d` (CI run
43918105805, job 133477890224). There robigo-rt runs in CI's Rust image
and the committed GNU R numbers come from the oracle image, both Debian
trixie (glibc 2.41).

The hand run pairs robigo-rt on this host (glibc 2.44) with GNU R in the
image (glibc 2.41), so it is labelled host-vs-image. The two agree to the
instruction on plain, defaults and named. On env-fn, superassign and s3
they differ. The likely cause is that the host's glibc dispatches
differently, but that is not established.

| row | GNU R 4.6.2 instr (oracle image) | robigo-rt, `perf-rt` (CI) | ratio | robigo-rt, host-vs-image | ratio | bar | | first run (host-vs-image) |
|---|---:|---:|---:|---:|---:|---|---|---:|
| plain | 4,800 | 1,923 | 0.401x | 1,923 | 0.401x | ≤ 0.6x | met | 0.42x |
| defaults | 6,088 | 2,000 | 0.328x | 2,000 | 0.328x | ≤ 0.6x | met | 0.34x |
| named | 4,639 | 1,849 | 0.399x | 1,849 | 0.399x | ≤ 0.6x | met | 0.42x |
| env-fn (`e$f`) | 3,832 | 2,593 | 0.677x | 2,508 | 0.654x | ≤ 1.0x | met | 0.67x |
| superassign (`<<-`) | 7,838 | 5,144 | 0.656x | 4,931 | 0.629x | ≤ 1.0x | met | 0.64x |
| s3 | 19,213 | 19,043 | 0.991x | 18,750 | 0.976x | ≤ 1.0x | met, 0.9% margin | 1.13x |
| trycatch (M3's row) | 93,888 | | | 114,543 | 1.220x | M3 | — | 1.32x |

- **Against the local build,** the hand run's ratios are 0.430x, 0.354x,
  0.417x, 0.693x, 0.659x, **1.014x** and 1.295x. On that baseline s3
  would not be met.
- **s3's path, 21,708 to 18,750 since the first run:**
  - X/P1's call path: #717, #719 and #728;
  - S1's dispatch:
    - #718, dispatch from slot frames;
    - #726, the methods' argument match plans;
    - #731 (ALIAS), S3 methods sharing their generic's argument slots,
      with no promise per argument.
  - Against those, #725 makes an S3 method's write to its argument in
    place, as GNU R does: an exactness fix with an accepted cost of 3.1%
    on s3.
- **Wall time, under §13's rule:**
  - **Method:** `calls.R` one process, at least 10 interleaved runs per
    runtime, each batched to at least 0.05 s, medians compared, and a 2%
    tie rule. `rivals.py` takes the fastest of N runs per runtime in turn,
    so `data/…/wall13.py` wraps its runner. It ran 11 rounds, rotating the
    order. Every `calls.R` process takes longer than 0.05 s, so the batch
    is one. Each runtime printed GNU R's answer.
  - **GNU R** is the 4.6.2 oracle image, §13's rival. `rivals.py` uses
    4.5.0.

  | runtime | median of 11 | range | robigo-rt ÷ | verdict |
  |---|---:|---:|---:|---|
  | robigo-rt | 0.321 s | 0.273-0.381 s | | |
  | GNU R 4.6.2 (image) | 0.305 s | 0.267-0.344 s | 1.050 | not established |
  | Ř | 1.051 s | 0.925-1.204 s | 0.305 | faster |
  | FastR | 9.159 s | 7.484-10.729 s | 0.035 | faster |

  Below Ř's and FastR's, outside the tie, so E1's wall-time bar is met.
  E1 asks only for Ř and FastR. Against GNU R, robigo-rt's 1.050x is
  within this run's noise: its own samples range from 0.273 s to 0.381 s
  at a load average of 4-9. So the run shows neither that robigo-rt is
  slower than GNU R nor that it is not. Caveats:
  - **Not the PGO build.** robigo-rt has no PGO build path:
    `scripts/pgo-build.sh` profiles and builds only the old runtime
    (`-p robigo-cli`). The binary measured is the plain release build.
    §13 asks for the shipped PGO build. The lead accepted the non-PGO
    binary for M1 as an exception, and §13's full protocol, PGO included,
    applies from M3 (`2026-11-16-m1-exit-lead-decisions.md`, (a)).
  - **A busy machine.** Other agents built on it during the run (load
    average 4-9 on 24 threads). The interleaving reduces the load's bias
    between runtimes, but the absolute times are above the first run's
    fastest-of-5 (`rivals.py`: robigo-rt 0.278 s, GNU R 4.5.0 0.230 s,
    Ř 0.873 s, FastR 6.56 s).
  - **No warmed peak.** It cannot be measured: `rivals.py`'s peak harness
    times itself with `proc.time()`, which robigo-rt's base does not have.
- **Proof:** `perf-rt` on every PR for the instructions; wall time by
  hand (`data/…/wall13.json`).
- **Owners:** S1 and X/P1.

### E2: fib, binarytrees

> fib, binarytrees: ≤ today's robigo (JIT off) in instructions and
> estimated cycles; binarytrees peak RSS ≤ today's and ≤ GNU R's.

**Not met:** fib, by the lead's decision (2026-11-16,
`2026-11-16-e2-fib-decision.md`): unmet in M1, owed to the native tier for
hot pure closures (G8).

The enforced numbers are `perf-rt`'s on main `c536597d` (job
133477890224). Estimated cycles are measured by hand only. Today's robigo's
numbers (G1) were measured on this host.

| row | today's robigo (G1) | robigo-rt, `perf-rt` (CI) | ratio | robigo-rt, host (hand) | ratio | | first run (host) |
|---|---:|---:|---:|---:|---:|---|---:|
| fib instructions | 8,428,736 | 10,722,529 | 1.272x | 10,722,705 | 1.272x | not met | 1.43x |
| fib est. cycles | 8,465,081 | | | 10,820,994 | 1.278x | not met | 1.44x |
| binarytrees instructions | 4,138,805 | 3,476,183 | 0.840x | 3,475,942 | 0.840x | met | 0.93x |
| binarytrees est. cycles | 4,388,485 | | | 3,610,727 | 0.823x | met | 0.91x |
| binarytrees peak RSS | 32,156 KiB (GNU R 42,060) | 23,704 KiB | 0.737x (0.564x) | 26,472 KiB | 0.823x (0.629x) | met | 0.82x (0.62x) |

- **fib:**
  - **Progress:** #722 and #727 (X/P1) brought it from 1.43x to 1.27x.
  - **The cause of the rest** (`2026-11-16-p1-fib-entry.md`): robigo-rt
    forces arguments lazily, with GNU R's evaluation counts, as GNU R
    does; that costs about 245 instructions a call. Today's robigo
    evaluates them eagerly at the call site.
  - **The cuts left** in that entry's profile reach about 1.14x, not
    1.0x.
- **binarytrees peak RSS:**
  - **The figures:** CI's 23,704 KiB is 2.9% above D5's 23,040 KiB
    (`d5-recycle.md:104-106`, `060c20af`, a machine that entry does not
    name). The hand run on this host (`rss.py --runs 5`) gives 26,472 KiB,
    11.7% above CI's.
  - **What that suggests:** the rise the first run could not explain may
    be this host's (WSL2, glibc 2.44) rather than robigo-rt's. That is
    not established.
- **Two references for "today's robigo":** G1's binarytrees figure of
  4,138,805 (`../g3.md:451`) and P1's 4,110,191 (`p1-base.md:31`) differ
  by 0.7%. Both bars are met either way.
- **Proof:** `perf-rt` on every PR for instructions and peak RSS;
  estimated cycles by hand.
- **Owners:** G8 for the bar; P1 for the cheap cuts.

### E3: determinism

> the stress script misses nothing in 30 ASLR runs per tier;
> `weak-keys-collections.R` passes; gctorture and `ROBIGO_GC_MOVE=1`
> clean.

**Met.**
- **Stress script:** `rt-stress.sh --runs 30`, ASLR 2. Eval 30 of 30 and
  Bc 30 of 30 runs missed nothing (`data/…/stress30.txt`). This is the
  third 30-run check, after `t1-collection.md:15-32` and the first run.
- **`tests/rt/capi/weak-keys-collections.R`:** passed in every
  conformance step of the final rt-ci.
- **gctorture and the moving collector:** both conformance steps pass
  293/294, production's 294/295 less `sort/radix-volume.R`, which the two
  gctorture steps skip by its `.requires`. The crate tests under
  gctorture, and under gctorture with `ROBIGO_GC_MOVE=1`, pass.
- **CI on every PR:** "conformance under gctorture (E3)", "conformance
  under ROBIGO_GC_MOVE with gctorture (E3)", and the crate-test steps for
  both. The stress script runs 3 times per profile there, not 30.
- **Gaps:** `sort/radix-volume.R` skips both gctorture steps by design
  (its 7,200 orders); `radix-random.R` covers its paths there.

### E4: overshoot

> bytes allocated between `GC_DUE` being set and the collection that
> serves it never exceed max(32 MiB, 10% of the live heap) on the slices
> M1 runs; peak RSS ≤ GNU R's on the memory benchmarks. Extended to every
> corpus and package suite as coverage grows.

**Met on the slice M1 runs.**
- **Overshoot:** `rt-overshoot.py`, in rt-ci: 314 programs in both
  profiles, 628 runs, none over the bound. The largest is 0.2384 of the bound:
  `benchmarks/rt/mem/churn.R`, 8,000,032 bytes against 33,554,432
  (`data/…/overshoot.md`), the first run's largest too (304 programs).
- **Peak RSS,** against GNU R in the oracle image
  (`2026-11-16-bars-oracle-baseline.md`). The enforced figures are
  `perf-rt`'s (CI, `rss.py --runs 3`); the host's are the hand run
  (`rss.py --runs 5`).

  | program | GNU R KiB | robigo-rt, `perf-rt` (CI) | ratio | robigo-rt, host | ratio | first run (host) |
  |---|---:|---:|---:|---:|---:|---:|
  | `benchmarks/suite/binarytrees.R` | 42,060 | 23,704 | 0.564x | 26,472 | 0.629x | 0.62x |
  | `benchmarks/rt/mem/listgrow.R` | 54,160 | 45,352 | 0.837x | 45,484 | 0.840x | 0.84x |
  | `benchmarks/rt/mem/churn.R` | 134,392 | 73,960 | 0.550x | 73,928 | 0.550x | 0.55x |

- **CI on every PR:** "overshoot on every slice (E4,
  scripts/rt-overshoot.py)" in `rt`; peak RSS in `perf-rt`.
- **Gap:** the extension to every corpus and package suite has not
  started: robigo-rt runs no corpus yet.

### E5: torture identity

> under `ROBIGO_NO_INPLACE=1` and `ROBIGO_MATERIALIZE=1`, semantic values
> and effects are identical on the prototype cases where the debug
> transformation is unobservable. […] And `getBindingType` matches the
> GNU R oracle before and after the first read for a constant, a symbol,
> a call and a defaulted argument, on a closure's first call and after
> GNU R's JIT threshold.

**Met.**
- **Both modes,** against production's 294/295:
  - **`ROBIGO_NO_INPLACE=1`: 292/293.** The mode does not run two cases.
    Each has a `.requires` that names `inplace-identity`, because its
    recording depends on writes made in place:
    - `capi/inplace-identity.R`;
    - `capi/dispatch-inplace.R`, added by #725.

    So 295 − 2 = 293 cases run and 294 − 2 = 292 pass.
  - **`ROBIGO_MATERIALIZE=1`: 294/295.** No case's `.requires` names
    `promise-identity`, so all 295 run.

  In each mode the one failure is `args/binding-types.R`.
- **The crate tests under each mode,** both passing:
  - **`ROBIGO_NO_INPLACE=1`:** the release tests of all six rt crates
    (`robigo-heap`, `robigo-heap-derive`, `robigo-rt`, `robigo-rtapi`,
    `robigo-rtbuiltins`, `robigo-rtcore`; `scripts/rt-ci.py:138-139`).
  - **`ROBIGO_MATERIALIZE=1`:** `robigo-rt`'s tests only
    (`scripts/rt-ci.py:150-151`).
- **Production-mode expectations:**
  - `crates/robigo-rt/tests/attrib.rs` and `tests/subset.rs`;
  - `tests/rt/capi/inplace-identity.R` (`t2-observation.md:96-139`);
  - `tests/rt/capi/dispatch-inplace.R` (#725).
- **`getBindingType`:** `tests/rt/args/binding-type-matrix.R` passes in
  every conformance step. It covers a constant, a symbol, a call and a
  defaulted argument, before and after the first read, on the AST's
  first call and in byte code from the second (`t2-observation.md:39-45`).
  - **`binding-types.R`,** the older case, still fails.
    `makeActiveBinding` is bound since #730, but `pi` is not: the case
    names it only inside `quote()`. Its stdout matched GNU R with a
    static scan that binds `pi`, which #730 tried and left out.
  - Its `try()` lines need regex (open items).
- **CI on every PR:** the two "conformance under …" steps and the
  crate-test steps under each mode.
- **Owner:** T2.

### E6: the API

> three builtin modules (arithmetic, `paste`/`nchar`,
> `assign`/`get`/`exists`) with no `unsafe`, reviewed for ergonomics by a
> council member who did not write them; the capi cases `eval`,
> `pairlist`, `unwind`, `extptr` pass over the minimal `robigo-rtapi`;
> the hook table's indirect-call cost (§10) recorded on `lapply` and `[`
> dispatch; regression cases pass for a package ALTREP Length callback
> that allocates and re-enters R, in-place mutation of BODY/FORMALS
> children, and mutation during a callback while compiled code is
> suspended.

**Met.**
- **Modules:** `arith.rs` with `relop.rs` and `logic.rs`; `paste.rs` and
  `character.rs`; `envir.rs` (`b1-builtins.md:14-19`). The crate is
  `#![forbid(unsafe_code)]` (`crates/robigo-rtbuiltins/src/lib.rs:24`).
- **Reviews:**
  - Sol: "pass with changes" (`e6-ergonomics-review.md:3-5`), done in
    R1c;
  - Gemini: "pass with changes" (`e6-review-gemini.md:3-8`), done in R1d;
  - Muse's review failed with an access error
    (`b1-builtins.md:185-189`).
- **capi cases:** `eval`, `pairlist`, `unwind` and `extptr` pass in every
  conformance step (`a1-capi-cases.md:26-31`). Two differ from the
  corpus: the rt variant of `pairlist` leaves out `capi_car_as_call`, and
  `extptr` uses `UNPROTECT(6)`.
- **Regression cases,** passing in every conformance step:
  - `capi/altrep-length.R`;
  - `capi/closure-mutation.R`;
  - `capi/callback-mutation.R`, with `capi/suspended-body-write.R`
    (`t2-observation.md:46-94`, `t2b-suspended-deopt.md:56-75`).
- **Hook cost,** re-measured with the probe
  `crates/robigo-rt/tests/hook_cost.rs`
  (`2026-11-16-bars-oracle-baseline.md`, section 3), in instructions per
  iteration:

  | mode | through the table | called by name | table's cost |
  |---|---:|---:|---:|
  | `lapply` over 16 elements | 48,808.7 | 48,779.8 | +29.0 (1.8 per element, 0.06%) |
  | `[` dispatch on an object, no method | 3,217.5 | 3,227.5 | −10.0 |
  | `[` on a non-object (the pre-test only) | 180.0 | | |

  The two runs agree within 0.002%. As in the first measurement
  (+29.0 and −8.0), the table costs 1.8 instructions per `lapply`
  element. The dispatch path is cheaper than its direct twin, which
  differs in its code layout, so the table's cost there is below what the
  probe can resolve. B1's stand-ins had 5 per element and 10 per dispatch
  (`b1-builtins.md:151-170`).

### E7: strings

> `paste`/`match`/`unique` loops ≤ today's instructions, or the
> regression recorded in `docs/ledger/g3.md` with its cause and the
> lead's acceptance.

**Met:** `match` and `unique` pass, and `paste` passes through the
acceptance clause.

The enforced numbers are `perf-rt`'s on main `c536597d` (job
133477890224); the hand run is on this host.

| row | today's robigo (G1) | robigo-rt, `perf-rt` (CI) | ratio | robigo-rt, host | ratio | first run (host) |
|---|---:|---:|---:|---:|---:|---:|
| e7-paste | 49,821 | 96,145 | 1.930x (ceiling 2.0x) | 96,027 | 1.927x | 1.94x |
| e7-match | 114,413 | 99,064 | 0.866x | 99,016 | 0.865x | 0.87x |
| e7-unique | 181,394 | 126,766 | 0.699x | 126,542 | 0.698x | 0.71x |

- **The record:** `2026-11-15-e7-seq.md:67-93`. It gives the cause:
  - interning each result string costs about 426 instructions
    (`Cx::mk_char`);
  - `paste_all`'s per-piece work costs about 600 per element.
- **The acceptance:** the lead's, of 2026-11-15. It holds while
  `e7-paste` stays at or below 2.0x today's robigo, its bar's reference:
  99,642 instructions per iteration. The lead set this ceiling on
  2026-11-16, and `perf-rt` fails above it. The first run found two gaps
  in the record, a wrong date and no ceiling, and both are closed there.
- **Open:** the first run's 96,790 was 2.2% above `ppacct.md:105-110`'s
  94,701.4 (main `2c6075a5`). No entry measures what raised it.
- **Proof:** `perf-rt` on every PR, which also enforces the ceiling.
- **Owners:** P1 for paste; D10 for interning, at M3.

### E8: unwinding

> `on.exit` that catches its own error during an unwind returns the outer
> condition; `R_UnwindProtect` cleanup that catches an error resumes the
> original continuation; a C context's `cend` runs before the jump; all
> in every tier and across a C frame.

**Met in Rust; GNU R-recorded cases pending, blocking sign-off** (the
lead's decision (b)). For the two tiers M1 has, Eval and Bc, each Rust
test passed in the final rt-ci, in the release step and under gctorture,
the moving collector and the pin report:
- `on.exit`: `crates/robigo-rtcore/tests/contexts.rs`, and across a C
  frame `crates/robigo-rtapi/tests/capi.rs`;
- `R_UnwindProtect`: `robigo-rtcore/tests/native.rs`, and across a C
  frame `robigo-rtapi/tests/capi.rs`;
- `cend`: `robigo-rtcore/tests/native.rs`, and across a C frame
  `robigo-rtapi/tests/capi.rs`.

`capi.rs` runs both profiles (`PROFILES`), with its C in
`tests/support/capi_test.c`.
- **Gaps:**
  - **No GNU R recording.** The expected results are Rust assertions,
    written by hand. GNU R never recorded them, and no R case checks E8
    against GNU R. The lead decided that E8 gets GNU R-recorded cases
    now, and they block M1's sign-off
    (`2026-11-16-m1-exit-lead-decisions.md`, (b)).
  - **Not with inlining off.** The tests do not run under
    `ROBIGO_NO_INLINE=1`: no step of `scripts/rt-ci.py` sets it, and
    neither do the tests.
  - **Two tiers.** "Every tier" is Eval and Bc: M1 has no native tier.

### E9: allocation failure

> the M1 slices pass under a small `R_MAX_VSIZE`, and an injected
> allocation failure after a visible effect neither loses nor repeats the
> effect.

**Met on the rt slice.**
- **Small limit:** under `R_VSIZE=4M R_MAX_VSIZE=16M`, conformance is
  294/295, the one known failure. `tests/rt/alloc/vector-limit.R` has
  GNU R's messages verbatim.
- **Injected failures:**
  - **What CI enforces on every PR:** `rt-fail-alloc.py --points 400` on
    one script, `tests/rt/alloc/effects.R`. That is 400 points spread
    over its allocations (`scripts/rt-ci.py:178-179`). The final rt-ci
    made 400 runs, and none changed the output.
  - **What the one-off sweep covered:** T1's run on 2026-11-16, at
    `c076b56a` on `g3/t1-collection` (`t1-collection.md:76-102`).
    - Every one of `effects.R`'s 2,252 allocations was refused in turn.
    - With `--slice`, up to 40 points went into each of the 188
      ratcheted `tests/rt` cases of that time (the injector stops at a
      case's allocations): 6,917 runs.
    - No run changed the output.

    The sweep has not been re-run on this tree, and the slice has grown
    to 295 cases since.
- **CI on every PR:** "conformance under a small vector limit (E9)" and
  "injected allocation failures (E9, scripts/rt-fail-alloc.py)".
- **Gap:** "the M1 slices" is the rt slice only, since robigo-rt runs no
  corpus yet.

### Decisions M1 rests on

The lead's decisions, and the decision bars' verdicts (`../g3.md`, "M1
decision bars"). §15.2's table (`docs/g3.md`) names D1-D6, D2s and D9
for M1. It has no D7, and D8 (the vector header) is M2's.

| | decision | by | record |
|---|---|---|---|
| L8 | Pins, adopted only with export rooting (§3 item 6) and allocation parity: an export allocates only where GNU R's does | the lead, 2026-11-13 | `docs/g3.md` §1 (L8) and §15.1 |
| D1 | Collection at safepoints only. No fallback is built | its bar | `2026-11-16-d1-run.md:64-70` |
| D2 | Inline frames. The pairlist control came within 5% on the first run, so the lead decided. On P1's path the control is 16.5% to 21.7% slower | rules 1-3, then the lead, 2026-11-16 | `2026-11-14-d2-d3-decision.md:14-28`, `2026-11-16-p1-reruns.md:155-165` |
| D2s | The overflow area | its bar | `2026-11-15-d2s-decision.md:20` |
| D3 | 16-byte value-stack slots, at D2's inline frames | its bar | `2026-11-14-d2-d3-decision.md:22` |
| D4 | Delayed and forced argument slots. Adopted below the bar on the first run; kept after the re-run, 19.25% below the control on real calls | the lead, 2026-11-14 and 2026-11-16 | `2026-11-14-d4-decision.md:25-29`, `2026-11-16-p1-reruns.md:142-154` |
| D5 | LIFO frame recycling, judged on real calls. It reverses the bar's `none` | the lead, 2026-11-16 | `2026-11-14-d5-decision.md:20`, `2026-11-16-p1-reruns.md:186-200`, `2026-11-16-d5-recycle.md` |
| D6 | Deferred link counts. `eager` is a control and never adoptable | its bar | `2026-11-14-d6-decision.md:15` |
| D9 | Dead registers cleared at last use, by the tie band | its bar | `2026-11-15-d9-decision.md:15` |
| E7 | `paste` is an accepted regression. It holds while `e7-paste` stays at or below 2.0x today's robigo, and `perf-rt` fails above that | the lead, 2026-11-15; ceiling 2026-11-16 | `2026-11-15-e7-seq.md:80-93` |
| E2 | `fib` is unmet in M1, owed to the native tier for hot pure closures (G8) | the lead, 2026-11-16 | `2026-11-16-e2-fib-decision.md` |
| E1 | Wall time: a non-PGO binary is accepted for M1. §13's full protocol, PGO included, applies from M3 | the lead, 2026-11-16 | `2026-11-16-m1-exit-lead-decisions.md`, (a) |
| E8 | GNU R-recorded cases now: the `on.exit` unwind case, and `R_UnwindProtect` and `cend` across a C frame. They block M1's sign-off, not this entry's merge | the lead, 2026-11-16 | `2026-11-16-m1-exit-lead-decisions.md`, (b) |
| cycles | A study of whether estimated cycles predict wall time across hardware better than instructions do. No gate change until the lead has read it | the lead, 2026-11-16 | `2026-11-16-m1-exit-lead-decisions.md`, (c) |

The bars themselves were revised once, by the foreman: GNU R's baseline is
the oracle image's, and env-fn is an M1 row at ≤ 1.0x
(`2026-11-16-bars-oracle-baseline.md`).

### Open items, with owners

Owners are the tracks the records name. Where a record names none, the
owner is the milestone of `docs/g3.md` §14.2 whose row covers the item,
or the design goal (`docs/design.md` §5.6: G4 packages, G6 `methods`/S4,
G8 the optimizing tier).

**Conformance.**
- **`args/binding-types.R`,** the one failing rt case.
  - **Why it fails:** `try()`'s error path calls `strsplit(msg, "\n")`,
    and with `fixed = FALSE` that is GNU R's TRE regex (`grep.c`).
    robigo-rt has no regex engine.
  - **Also needed:** a static scan of the case in `scripts/base-subset.R`
    that binds `pi`, which it names only inside `quote()`. With that
    scan the case's stdout matches (#730).
  - **Owner:** M4 (builtins, `grep.c`).

**Bars.**
- **E2 fib:** 1.272x. Owners: G8, and P1 for the cheap cuts
  (`p1-fib-entry.md`).
- **E7 paste:** under its 2.0x ceiling. Owners: P1 for paste, and D10
  for interning at M3.
- **E1's wall time under §13's rule:** applied, with interleaved medians
  over 11 rounds (E1). The lead deferred the rest to M3 (decision (a)):
  - the PGO build;
  - the warmed peak, which cannot be measured without `proc.time`.

  Owner: M3, when E1 gains its M3 rows.

**Deferred.**
- **Weak cache words:** wait for the first inline cache that holds an
  object pointer. All 16 of the heap's `extra` bits are taken, so they
  need another marker. Owners: C1, with M2's layout.
- **The JIT entry word:** owner G8.

**Verification modes without a CI job.** `docs/g3.md` §16 said every mode
in its table had one; that sentence is corrected in this PR.
- **`ROBIGO_BUILTIN_DIFF=1`:** a doc comment only
  (`crates/robigo-rtbuiltins/src/registry.rs:99`). M4's builtins run
  under it, by its row. Owner: the B track.
- **`ROBIGO_REF_EVAL=1`:** does not exist. Owner: the V track, by M3, the
  one engine.
- **Poisoning dead slots:** no switch exists. The same row's frame
  quarantine has its CI job. Owner: the V track, which clears dead
  registers (D9).

**Blocking M1's sign-off.**
- **E8's GNU R-recorded cases:** the `on.exit` unwind case as a
  `tests/rt` R case, and `R_UnwindProtect` and `cend` across a C frame
  with their C built for GNU R. All run in every profile and under
  `ROBIGO_NO_INLINE=1`. The lead's decision (b); branch
  `g3/a1-e8-oracle`.
- **C1c, the link-count fix:** owner C1, branch `g3/c1c-link-walk`.
  - **The bug:** robigo-rt counts links for every value-stack slot,
    builtins' Rust roots included.
    - **Where:** `crates/robigo-rt/src/links.rs`, through
      `Cx::stack_links` (`crates/robigo-rtcore/src/raw.rs:203`).
    - **What GNU R does:** `INCLNK_stack_commit` (`eval.c:997-1016`)
      counts only the byte-code node stack's values.
    - **What it changes:** a value that GNU R modifies in place is copied
      instead. For example:

          x <- c("a","b","c")
          g <- function(...) .Internal(vapply(x, function(e) { x[2] <<- "Z"; 1 }, 0, TRUE))
          print(g())

      GNU R 4.6.2 prints the names `a Z c`, and robigo-rt on `c536597d`
      prints `a b c` in every profile, because `do_vapply`'s Rust root
      counts as a link. Reproduced on `c536597d`'s binary and the 4.6.2
      oracle.
  - **The fix, by C1's report:** with it, the `vapply` case prints
    `a Z c` as GNU R does and is ratcheted, and rt-ci and `perf-rt` pass.
    It has no pull request yet.

**Settle before M2/M3 planning.**
- **The baseline gap.** The oracle image's GNU R counts 3.9% to 12.9%
  more instructions than the local build, and nobody has explained why.
  The likely causes are glibc 2.41's CPU dispatch against the host's
  2.44, and Debian's build flags (`2026-11-16-bars-oracle-baseline.md`).
  E1's s3 verdict depends on it (question 3). No owner yet.
- **`perf-rt`'s CPU and baseline policy:**
  - print the runner's CPU (the job prints none today);
  - report estimated cycles without failing on them;
  - re-measure GNU R on CI's runner class at each milestone, not on one
    developer machine.

  No owner yet. The lead asked for a study first (decision (c)): whether
  estimated cycles predict wall time across hardware better than
  instructions do. It comes after E8's cases, with its own entry.
- **The eager-evaluation proof for fib:** that evaluating a nested call's
  leaf arguments early is unobservable (`p1-fib-entry.md`, "The gap").
  It is the other way to E2's bar besides G8. It needs an owner, or a
  decision to decline it.

**Stop-gaps: "not implemented yet" in robigo-rt,** re-checked on
`c536597d`.

robigo-rt raises "robigo: X is not implemented yet" where it knows it
differs from GNU R.
- **The count:** the four rt crates hold 198 distinct such messages, the
  same set as on the first run's `2671f453`. Line numbers moved, and the
  list below is `c536597d`'s.
- **The paths** are shortened: `bi` is `crates/robigo-rtbuiltins/src`,
  `rt` is `crates/robigo-rt/src`, `core` is `crates/robigo-rtcore/src`,
  and `api` is `crates/robigo-rtapi/src`.

| area | what | where | owner |
|---|---|---|---|
| loader stubs | 3 primitives (`call`, `ceiling`, `exp`) and 8 internals (`grep`, `iconv`, `pipe`, `rapply`, `rawToChar`, `regexpr`, `sub`, `which.max`), each checked on this binary | `rt/base.rs:270-317` | M4 |
| names outside the bound | `round`, `withVisible`, `R.home` and `proc.time` give "could not find function" (`makeActiveBinding` is bound since #730) | `proto/base-bound.txt` | M4 (M3 for the boot from databases) |
| size limits | a function frame of more than 65,536 variables; an environment of more than 65,536 past its frame's shape; more than 65,535 formals or call arguments | `core/env/slotapi.rs:310`, `core/env/mod.rs:1681`, `rt/args.rs:1103-1105` | N1 (frames), C1 (arguments) |
| environments | user-defined databases (`core/env/mod.rs:371`); `SET_FRAME` on a closure frame (`core/env/slotapi.rs:466`); `eval()` in an S4 environment or an untagged pairlist (`rt/eval.rs:648,678`); `as.environment` of several positions, S4, a list (`bi/envir.rs:1094,1130,1131`); a frame number as `envir` (`bi/envir.rs:274`) | as listed | N1; the S4 cases G6 |
| collation | text not in FCD form, including the Tibetan vowels U+0F73, U+0F75 and U+0F81; `R_ICU_LOCALE` other than C | `bi/text.rs:104-109` | the collation track (`2026-11-16-collation.md`) |
| sort | radixsort grouping with `na.last = NA`; a `decreasing` that is not logical; `.Internal(rank())` past the vector | `bi/sort/radix.rs:178,196`, `bi/sort.rs:799` | the sort track (`2026-11-16-sort.md`) |
| package ALTREP | writing, duplicating or reading in place a package ALTREP object, refused in about 60 places: arithmetic, `paste`, subsetting and sort (`bi/arith.rs:42`, …), `for` over an ALTREP sequence (`rt/vm.rs:1442`), the C API's `DATAPTR`, `*_ELT` and `SET_*` (`api/api.c:226-436`) | as listed | M5 (packages) |
| options | 11 options cannot be set to anything but NULL (`width`, `digits`, `deparse.cutoff`, `expressions`, `nwarnings`, `max.print`, `keep.source`, `echo`, `verbose`, `warnPartialMatchDollar`, `warnPartialMatchAttr`); 13 more lack their checks and C state | `bi/options.rs:337-349,418,692` | M4 |
| deparse | S4 objects, unforced promises, srcrefs, `S_compatible`, `hexNumeric`, `digits17` | `rt/deparse.rs:588-1414` | M4 |
| printing | matrices, arrays, array-like lists and pairlists, primitives, `row.names` | `rt/print.rs:231-710` | M4 |
| S4 | S3 dispatch on an S4 object; S4 dispatch and group dispatch; two different `Ops` methods but for the `difftime` pairs | `rt/dispatch.rs:181,2516,2637,2669,2870` | G6 |
| subscripts | matrix and array subscripts by dimnames; `[[<-` on a pairlist | `bi/subset.rs:456`, `bi/subassign.rs:681,968` | M4 |
| namespaces | `loadNamespace`, and every namespace but base, so no package and no `stats` (`rank(ties = "random")`) | `rt/ns.rs:68,71` | M3 (base boot), M5 (packages) |
| evaluator | byte-code objects (`BCODESXP`, marked M3); internal restarts; `gc(verbose = TRUE)`; `tracemem` | `rt/eval.rs:117`, `rt/conditions.rs:843`, `rt/memory.rs:32`, `core/edit.rs:157` | M3 |
| C API | `R_init_<pkg>` routine registration; `.Call` of a native symbol reference; `R_signal_protect_error`; some `SETCAR`/`SETCDR`/`SET_TAG` cases; and one assert, a panic rather than an R error, for `on.exit` code in a context C pushed | `api/dotcode.rs:176,323`, `api/errors.rs:105`, `api/api.c:410-436`, `core/context.rs:1722` | M5 |

**Deferred follow-ups** from the ledger, still open:
- **Memory and frames:**
  - D4's extra REALSXP from early values (`p1-reruns.md:146-149`).
    Owner: P1.
  - The 3 LISTSXP of `eval(quote(v <- 1))`, never explained
    (`d2s-runs.md:100-105`). Owner: P1, which that entry leaves it to.
  - Deopt only at statement boundaries (`t2b-suspended-deopt.md:50-53`).
    Owner: T2.
  - The binarytrees peak-RSS rise since `060c20af` (E2). It may be this
    host's: CI's figure is 2.9% above D5's, and the host's 11.7% above
    CI's. Owner: T1.
- **Compiler** (`v2b-cmp.md:89-97`). Owner: V2, M3 for BCODESXP.
  - Not ported: the `::`/`:::` handler, DOTCALL, `cmpIs`, `cmpPrim1/2`.
  - Promise code is kept per expression, not per promise.
  - A closure with very many formals recompiles on every call
    (`p1-inline.md:171-175`).
- **Collation** (`2026-11-16-collation.md`). Owner: the collation track.
  - Non-FCD text and `R_ICU_LOCALE` (`:189`).
  - A Latin fast path (`:166-168`).
  - An `icu_collator` debug assertion that the dev profile turns off. It
    has not been reported upstream, which needs Max's approval
    (`:172-179`).
- **The base subset** lacks `round`, `sys.calls`, `withVisible`, `debug`,
  `body`, `enc2native` and `proc.time` (`p1-unboxed.md:90-93`). Several
  cases and tools work around them; the rivals' warmed peak is one.
  Owner: M4.
- **C API.** Owner: M5.
  - `on.exit` in a C-pushed context, and `R_run_onexits(NULL)` at
    `R_CleanUp` (`c2-contexts.md:187-190`).
  - Protections on paths no case reaches yet (`ppacct.md:160-163`).
  - Over-unprotecting stays robigo's error, the foreman's interim
    decision (`ppacct.md:157-159`).
  - No AArch64 stub (`h3-debug-gc.md:161-163`).
  - 34 of the 75 capi C sources do not compile against 4.6.2's headers
    (`oracle-462.md:70-87`).
- **Oracle:** 4.6.2's behaviour changes still need a minimized
  reproduction and a NEWS entry (`oracle-462.md:57-63`). No owner is
  named; §14.3 asks for one with each recorded change.
- **Undecided, for the lead:**
  - `R_HOME` and `R.home()` (`startup-options.md:61-64`);
  - the §8 TRUELENGTH shared-bit rule (`../g3.md:139-143`).
- **Sort** (`2026-11-16-sort.md:207-217`). Owner: the sort track.
  - `rank(ties = "random")` needs `stats`.
  - `round()` is missing.
  - An error reached through `NextMethod` to a special names argument
    values where GNU R shows expressions.

Done since the first run, so not open:
- **C1b, the code object's layout and invalidation:** closed. It was
  delivered by V2's #664 (closure duplicate, JIT state), and by T2's #690
  and T2b's #691 (code floors, tracked setters, BODY/FORMALS
  invalidation, suspended deopt). The code object is
  `crates/robigo-rt/src/code.rs`.
- **The four env cases:** pass, with `makeActiveBinding` in the base
  subset (#730).
- **s3 under its bar** (#731), and the E7 acceptance's date and ceiling
  (`2026-11-15-e7-seq.md`).
- **The bars:** GNU R 4.6.2's baseline from the oracle image, env-fn's
  bar and E6's hook cost (`2026-11-16-bars-oracle-baseline.md`), and the
  `perf-rt` gate (`2026-11-16-m1-bars-gate.md`).
- **Earlier entries:**
  - collation's missing sort internals (`sort.md`) and its short options
    list (`startup-options.md`);
  - the `.Call` stub's zeroing (removed in `a1-capi-cases.md`);
  - PPACCT's resumed-loop gap (`rtapi-sweep.md:28-38`);
  - D5's cause and the D2–D5 decisions (`d5-709.md`, `p1-reruns.md`);
  - the suspended-body deopt (`t2b-suspended-deopt.md`);
  - superassign and binarytrees under their bars
    (`p1-superassign.md:91`, `p1-bt-check.md:75-77`);
  - the E6 reviews (`e6-ergonomics-review.md`, `e6-review-gemini.md`);
  - D9 (`d9-decision.md`);
  - weak-keys in the rt slice (`a1-capi-cases.md:5`).

### Questions for the council

The judgment calls a skeptical reader should push on:

1. **Two E criteria pass by decision, not by measurement.**
   - **fib (E2):** unmet at 1.27x. The gap is GNU R's forcing semantics
     (`p1-fib-entry.md`), and the only scheduled fix is the native tier,
     G8. Proving eager evaluation unobservable is the other way to 1.0x,
     and no one owns it.
   - **paste (E7):** accepted at 1.930x in `perf-rt` under a 2.0x
     ceiling, which leaves 3.6% of headroom until D10 at M3.

   Should M1 exit with E2 half met, or should the bar be rewritten?
2. **The gate measures instructions and peak RSS only.**
   - **Hand runs:** E1's wall time and E2's estimated cycles are measured
     by hand. The wall time follows §13's rule now, but on a non-PGO
     binary and a shared machine.
   - **Instructions miss memory traffic:** D5's recycling costs 0.1% to
     0.6% in instructions and saves 4% to 6% in estimated cycles
     (`p1-reruns.md:186-192`).
   - **One CPU:** the committed GNU R numbers come from one CPU, and CI's
     runners vary, so a bar's ratio moves with the runner. The policy to
     settle is in the open items.

   Is an instruction gate enough to hold bars that speak of cycles and
   seconds?
3. **The baseline decides E1's last row.**
   - **The enforced figure:** s3 is 0.991x in `perf-rt`, with GNU R from
     the oracle image and robigo-rt in CI's image, a 0.9% margin.
   - **The hand runs:** 0.976x host-vs-image, and 1.014x against the
     local build the oracle image replaced, which counts 3.9% fewer
     instructions on s3.
   - **Unexplained:** nobody has established why the oracle image counts
     3.9% to 12.9% more.

   Is E1 met by robigo-rt, or by the choice of baseline?
4. **The bars predate inlining.**
   - **The rows run inline:** the decision rows bind no arguments, so D4
     and D5 went to the lead. On real calls the bars hold, and on the
     rows themselves they do not decide.
   - **The protocol was not broken:** both decisions followed its
     failed-premise rule.

   Should M3's bars be written on real calls?
5. **"Met" means met on the rt slice.**
   - **What runs:** robigo-rt runs 188 base closures and its own
     conformance cases (295, of which 294 pass), and no corpus yet.
   - **E4 and E9** hold on that slice, and E5's identity is checked on the
     prototype cases.
   - **What the slice missed:** the link-count bug (C1c, open items) changes a
     value that GNU R modifies in place. A hand-written case found it,
     after every step of the slice passed.

   Is that coverage enough to start M2 and M3 on, or should a corpus
   slice be an exit condition?
