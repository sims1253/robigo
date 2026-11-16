## 2026-11-16: M1 bars revised: GNU R 4.6.2 from the oracle image, env-fn among the 1.0x rows; E6's hook cost with the real hooks

Three gaps the M1 exit evidence (`2026-11-16-m1-exit-evidence.md`) found
and could measure, closed at the foreman's request. Two revise the bars
entry (`../g3.md`, "M1 decision bars" and "M1 baselines"), as its protocol
asks: a dated section that names the bar it replaces and why
(`../g3.md:156-160`). The bars it replaces stand for the runs and
decisions that cited them.

- **Tree:** main `d0052bfe` (#720 merged). The robigo-rt numbers recomputed
  below are the evidence entry's, from main `2671f453`.
- **Machine:** the evidence entry's (AMD Ryzen 9 5900X, WSL2 Linux 6.18).
- **Raw data:** `data/2026-11-16-bars-oracle-baseline/`.

### 1. GNU R 4.6.2's baseline from the oracle image

G1's GNU R column (`../g3.md:419-421`) was measured on a local build,
`~/.cache/robigo-research/R-4.6.2-build`, and marked provisional: "re-run
on M0's 4.6.2 oracle image once it lands". No entry did that re-run until
now.

- **Image:** `robigo-oracle:4.6.2` (R 4.6.2, Debian glibc 2.41) with
  Debian's valgrind 3.24.0 installed in a throwaway container. That image
  makes the conformance recordings.
- **Command:** inside the container, twice:

      RSCRIPT_BIN=/usr/lib/R/bin/exec/R R_HOME_DIR=/usr/lib/R \
        scripts/idiom-vs-r.py --rows benchmarks/rt/calls-rows.toml \
        --cycles --robigo /bin/true --json OUT

  This is the bars tool's own N against 2N under callgrind. `/bin/true`
  stands in for robigo, whose columns are not used.
- **Reproduction:** the two runs agree within 0.0001% in instructions and
  0.000% in estimated cycles, on every row.

The image's counts are 3.9% to 12.9% above the local build's on every row.
The likely causes are the C library (2.41 against the host's 2.44, whose
CPU dispatch differs) and Debian's build flags. The cause is not
established here.

| row | local build instr | oracle image instr | ÷ | oracle image est. cycles |
|---|---:|---:|---:|---:|
| plain | 4,472 | 4,800 | 1.073 | 5,536 |
| defaults | 5,643 | 6,088 | 1.079 | 6,971 |
| named | 4,433 | 4,639 | 1.047 | 5,420 |
| env-fn | 3,619 | 3,832 | 1.059 | 4,219 |
| superassign | 7,480 | 7,838 | 1.048 | 9,159 |
| s3 | 18,497 | 19,213 | 1.039 | 25,334 |
| trycatch | 88,471 | 93,888 | 1.061 | 110,816 |
| fib | 22,723,893 | 24,056,138 | 1.059 | 24,546,155 |
| binarytrees | 3,571,751 | 3,781,021 | 1.059 | 3,936,091 |
| e7-paste | 177,406 | 195,871 | 1.104 | 220,090 |
| e7-match | 191,681 | 207,795 | 1.084 | 271,146 |
| e7-unique | 151,313 | 170,862 | 1.129 | 203,407 |
| d2s-assign | 13,029 | 13,588 | 1.043 | 18,345 |
| d2s-eval | 16,146 | 16,902 | 1.047 | 21,767 |
| d2s-callee-assign | 15,434 | 16,158 | 1.047 | 20,880 |
| d2s-delayed | 16,513 | 17,575 | 1.064 | 18,916 |

**What changes:**
- **The baseline:** from this entry on, the oracle image's column is GNU R
  4.6.2's baseline for E1, and for every bar G1's GNU R column set. That
  replaces the local build's.
- **E1's thresholds**, in instructions:

  | | rows and thresholds |
  |---|---|
  | ≤ 0.6x | plain ≤ 2,880; defaults ≤ 3,653; named ≤ 2,784 |
  | ≤ 1.0x | env-fn ≤ 3,832; superassign ≤ 7,838; s3 ≤ 19,213 |

  env-fn's place is set in section 2 below.
- **What doesn't change:** E2 and E7 compare with today's robigo, so
  their bars stay.
- **Mixed C libraries:** robigo-rt is measured on the host's glibc, and
  GNU R in the image on its own. That mix is the price of measuring GNU R
  where the recordings come from. The bars protocol's rule against
  comparing two machines' counts (`../g3.md:209-211`) is about the
  machine and the C library's CPU dispatch. It applies to robigo-rt
  against robigo-rt, which stays on the host.

**Peak RSS:** GNU R in the image (`rss.py --runs 5` in
`scripts/oracle-run.sh 4.6.2`), the E2 and E4 baseline from this entry
on:

| program | local build KiB | oracle image KiB |
|---|---:|---:|
| `benchmarks/suite/binarytrees.R` | 43,588 | 42,060 |
| `benchmarks/rt/mem/listgrow.R` | 55,484 | 54,160 |
| `benchmarks/rt/mem/churn.R` | 135,960 | 134,392 |

### 2. env-fn is an M1 row at ≤ 1.0x

**What this replaces:**
- the bars entry's PLAIN set, "`plain`, `defaults`, `named`, `env-fn`
  (§13's plain-call idioms)" (`../g3.md:191`);
- G1's "env-fn ≤ 2,171" at ≤ 0.6x (`../g3.md:463`).

**Why:** `docs/g3.md` §15.3 names `e$f` among E1's M1 rows at ≤ 1.0x:
"M1 rows: plain, defaults, named, `e$f`, `<<-`, S3 ≤ 1.0x GNU R's
instructions, the plain-call idioms ≤ 0.6x". Neither §15.3 nor §13 lists
the plain-call idioms. The bars entry put env-fn in PLAIN by its own
reading, and P1's entries already judge env-fn at 1.0x (`p1-base.md`,
`p1-gaps.md:5,44`).

**From this entry on:**
- PLAIN is `plain`, `defaults`, `named`, at ≤ 0.6x.
- env-fn is an E1 row at ≤ 1.0x, with `superassign` and `s3`.
- CALLS, the six E1 rows, is unchanged.

**What stands:** D5's rule cited PLAIN's four rows (`../g3.md:329-331`),
and its decision stands as made.

**The rows file:** env-fn's `class` moves from `plain-call` to `m1` in
`benchmarks/rt/calls-rows.toml`. The class labels the row and no script
reads it. The row's setup, body and `n`, its workload, are unchanged, so
no measurement changes (the file's rule: a cited row's workload is never
edited).

### 3. E6: the hook table's cost with robigo-rt's hooks

E6 asks for "the hook table's indirect-call cost (§10) recorded on
`lapply` and `[` dispatch". B1 recorded it with stand-in hooks, before
any evaluator existed (`2026-11-13-b1-builtins.md:151-170`): 5
instructions per `lapply` element and 10 per dispatch. It left the re-run
to V1, and no entry did it.

**The probe:** `crates/robigo-rt/tests/hook_cost.rs`, ignored, has B1's
shapes with robigo-rt's own hooks installed:
- **`lapply`:** B2b's shape over 16 elements of a real closure,
  `function(x) x`. Each element is called through `cx.apply`, as
  `do_lapply` calls it, or through `robigo_rt::apply_hook` by name.
  `apply_hook` is now re-exported for this.
- **`[` dispatch:** on an object whose class has no method. It goes
  through `cx.try_dispatch`, or through the same pre-test and
  `robigo_rt::dispatch::dispatch_hook` by name.

A mode and its direct twin do the same work, so their difference is the
table's own cost.

**Measurement:** callgrind, N = 5,000 against 10,000, twice. The two runs
agree within 0.011% (`dispatch-plain`, 195.01 against 195.03). Instructions per iteration:

| mode | through the table | called by name | table's cost |
|---|---:|---:|---:|
| `lapply` over 16 elements | 51,853.8 | 51,824.8 | +29.0 (1.8 per element, 0.06%) |
| `[` dispatch on an object, no method | 3,316.5 | 3,324.5 | −8.0 |
| `[` on a non-object (the pre-test only, no call) | 195.0 | | |

**Reading:**
- **`lapply`:** the table costs 1.8 instructions per element, against
  B1's 5 with stand-ins. Each element's call costs about 3,240 on its
  own.
- **`[` dispatch:** the table path is 8 instructions cheaper than its
  direct twin, which differs in its code layout. The table's cost there
  is below what this probe can resolve.
- **Non-objects:** the pre-test spares them the call, as in B1.

E6's hook-cost sub-bar is recorded; E6 is met.

**Command:**

    ROBIGO_HOOK_COST=MODE:N valgrind --tool=callgrind \
      <the hook_cost test binary> --ignored --exact hook_cost --test-threads 1

MODE is one of `lapply-hook`, `lapply-direct`, `dispatch-hook`,
`dispatch-plain` and `dispatch-direct`; N is 5,000 and 10,000.
