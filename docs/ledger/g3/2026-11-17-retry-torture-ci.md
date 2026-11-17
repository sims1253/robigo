## 2026-11-17: the `ROBIGO_RETRY_TORTURE=1` CI step

`2026-11-17-retry-torture.md` brought the mode in (#780) with no CI step:
five ratcheted cases failed under it. S1's #791 fixed their common root, an
unrooted on.exit list in rtcore's `end_at_stop` across
`R_FixupExitingHandlerResult`. This entry adds the step, as the foreman
decided.

### The step

`scripts/rt-ci.py` gains one step, last: "conformance under
`ROBIGO_RETRY_TORTURE=1` (E9: every build's retry runs once)".
- **One build with the feature.** It is the conformance test (`cargo test
  --release -p robigo-rtapi --features robigo-rtcore/retry-torture --test
  conformance`), so cargo builds robigo-rt with the mode compiled in for
  this step only.
- **Last, so every other step tests the binary users get.** cargo keeps
  both builds in the target directory.
- **The collector modes' case timeout, 600 s.** Under the mode alone, four
  cases take longer than the runner's 20 s:

  | case | seconds |
  |---|---:|
  | `tests/rt/sort/radix-volume.R` | 89 |
  | `tests/r/memory/finalizers-self-ref-envs.R` | 76 |
  | `tests/r/jit/loop-vectors.R` | 31 |
  | `tests/r/jit/loop-closed-form-super.R` | 21 |

  That is from timing each of the 472 ratcheted cases alone: 433 CPU
  seconds in all.
- **With `ROBIGO_GC_MOVE=1` as well** it takes about 20 minutes, so it is
  not a step: `docs/g3.md` §16 gives the command for the run by hand.
- **The `rt` job's timeout goes from 30 to 40 minutes,** from the
  measurements below.
- **`scripts/test-rt-ci.py` checks** that exactly one step builds with the
  feature, that it is the last, that it turns the mode on with the 600 s
  timeout, and that robigo-rtcore's manifest has the feature.

### On main with #791

The ledger's two numbers again, on main 229f54d6, where #791 is merged.
Run locally, with the runner's two jobs:

| run | time | passes |
|---|---:|---:|
| plain, for scale | 9 s | 509/516 |
| the step: `ROBIGO_RETRY_TORTURE=1`, as rt-ci.py runs it | 201 s, plus a 37 s feature build | 509/516 |
| by hand: `ROBIGO_RETRY_TORTURE=1 ROBIGO_GC_MOVE=1` | 1,243 s | 509/516 |

Again with main merged at filing (#794 in): the plain run 12 s, the step
231 s plus a 60 s feature build, both 522/532 (a busier machine than the
table's).

All three runs pass the same 509 of 516 cases, and no ratcheted case fails,
so the mode adds no failure on either collector. Under it, #780's three
failing cases pass, and with GC_MOVE too, its four. The foreman expected
"464/464 for both": the slice grew, and the result is the one meant, every
ratcheted case.

### The job's timeout

A hosted runner has 4 CPUs, so the step was also measured on 4 pinned CPUs.
Its build was the feature rebuild, robigo-rtcore and every crate above it,
with 4 build jobs. The step ran with this branch's binary under #804's
runner (`ROBIGO_RT_BIN`, `ROBIGO_CONF_JOBS`), against #804's 515 cases:

| | time |
|---|---:|
| the feature rebuild | 43 s |
| the step, 2 cases at a time | 229 s |
| the step, a case per CPU (#804) | 125 s |

The slowest hosted runners ran the gctorture conformance step in 150 s,
against 130 s on these 4 pinned CPUs: 1.15x. On such a runner the step
takes about 145 s with #804 and about 265 s without it. The rebuild is
compiling, not the runtime: at twice the time here it is under 1.5 min.
The job without this step takes about 20 min there with #804
(`2026-11-17-rt-job-capacity.md`) and about 22 without it. With the step
that is about 24 and 28 min, and 40 keeps 1.6x and 1.4x of those.

### #780's deferred OCR lows

The foreman deferred five low findings from #780's second OCR round into
this PR:
- **r4598852615:** `RefuseFirst` is `#[must_use]`.
- **r4598852658:** attempts cannot nest. A build's closure gets `&Cx`, and
  `build_with` and `retry_alloc` take `&mut Cx`, so the guard's disarm
  ends only its own attempt's refusal. The guard's doc says so.
- **r4598852682:** `set_retry_torture` asks the heap whether E9's knob is
  set, through `Core::fail_at`, the heap's own parse, armed or not yet. It
  no longer reads the environment again, and asks only when turning the
  mode on.
- **r4598852709:** `build_with` and `retry_alloc` share one helper,
  `Cx::refuse_once`.
- **r4598852728:** `tests/retry_torture_knob.rs` checks the refusal. It is
  its own test binary, because the knob's variable is process-wide and the
  sibling tests turn the mode on. It fails without the check.

### #794's deferred OCR low

- **r4600865695:** `scripts/base-subset.sh` prints where the reached-by
  report is before `--check`'s diffs. A failing check is when the report
  helps, and it used to print only on success.
