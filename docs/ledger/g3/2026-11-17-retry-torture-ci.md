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
- **The `rt` job's timeout goes from 30 to 45 minutes.** The job takes about
  20 minutes on main, and this step's feature build and run add about 10.
- **`scripts/test-rt-ci.py` checks** that exactly one step builds with the
  feature, that it is the last, that it turns the mode on with the 600 s
  timeout, and that robigo-rtcore's manifest has the feature.

### On main with #791

The ledger's two numbers again, on main 81eeb1ab with #791's head
(6e60a04e) merged in. Run locally, with the runner's two jobs:

| run | time | passes | ratcheted cases that fail |
|---|---:|---:|---:|
| plain, for scale | 10 s | 473/481 | 0 |
| the step: `ROBIGO_RETRY_TORTURE=1`, as rt-ci.py runs it | 178 s, plus a 33 s feature build | 473/481 | 0 |
| by hand: `ROBIGO_RETRY_TORTURE=1 ROBIGO_GC_MOVE=1` | 1,206 s | 473/481 | 0 |

The slice has 481 cases now, of which 473 are ratcheted, and every
ratcheted case passes in all three runs. The 8 that don't pass fail
without the mode too, so the mode adds no failure on either collector.
Under it, #780's three failing cases and, with GC_MOVE too, its four now
pass. The foreman expected "464/464 for both": the slice grew, and the
result is the one meant, every ratcheted case.

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
