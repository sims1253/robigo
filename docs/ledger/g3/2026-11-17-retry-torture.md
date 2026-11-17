## 2026-11-17: `ROBIGO_RETRY_TORTURE=1`, E9's refusal at every build

The foreman's decision, after the E9-under-GC_MOVE measurements below: an
rtcore debug mode, owned by A1, with no CI step until the cases it fails
pass.

### Why

A `&Cx` build (`Cx::build_with`, `Cx::retry_alloc`) runs its closure again
after a refused allocation, with a collection in between. Two kinds of
code break only on that retry:
- an object read before the build and used inside it: unrooted, it is
  freed; under `ROBIGO_GC_MOVE=1`, it has moved;
- an effect inside the closure, which the retry repeats.

E9's knob, `ROBIGO_FAIL_ALLOC_AT=n`, refuses one allocation per run, so a
script reaches such a bug only if `n` lands on the right allocation.

**#769's environmentName crash shows how rarely that happens.** I rebuilt
#769 just before its fix (ca1798cc) and swept every allocation with
`scripts/rt-fail-alloc.py`:
- under `ROBIGO_GC_MOVE=1`, `tests/rt/ns/registry.R` with the fix commit's
  own `environmentName(ns)` line differs at exactly one of 4,845
  allocations (#3621, a SIGBUS);
- `search.R` (5,937 allocations) and `registry.R` without that line
  (4,781) pass every point, and without GC_MOVE every point passes.

The script spreads K points evenly, and K = 40, 200, 400 and 1,000 all miss
#3621. On main (de83cade), a GC_MOVE step of `--slice --points 40` took
125 s locally for 11,648 runs and found nothing, and the median slice case
makes about 7,000 allocations. So a sampled step would catch a bug of this
class about K / allocations of the time. The foreman declined it.

### The mode

`ROBIGO_RETRY_TORTURE=1` refuses the first allocation of every
`build_with` and `retry_alloc` call once:
- the knob is set to refuse the next allocation before the first attempt;
- it is off again after that attempt, whether the attempt allocated or
  not;
- so each build's retry runs once, and nothing outside a build is refused.

`robigo-rt` reads it at `install`, as `ROBIGO_NO_INPLACE` is read. Embedders
and tests use `Cx::set_retry_torture`.

**It is compiled in only with robigo-rtcore's `retry-torture` feature:**
`cargo build --release -p robigo-rtapi --features robigo-rtcore/retry-torture`.
GNU R's `--enable-strict-barrier` works the same way.
- **Why:** as a plain runtime switch, its check in every build cost 3 to 4
  instructions per build. `perf.py rt-bars` measured that against main's
  binary at the same base (7a628603): +0.65% on s3 and +0.70% on
  binarytrees, which the perf gate fails (0.5%, no bypass), and +0.22% on
  plain.
- **Without the feature,** the check is a constant false and compiled out,
  and `ROBIGO_RETRY_TORTURE=1` is refused with the build command.

**It reaches no state the heap cannot.** Each refusal is E9's knob refusing
that allocation, which is what `ROBIGO_FAIL_ALLOC_AT=n` does at one `n`. The
heap refuses an allocation the same way when the request would pass a limit
(`poll_slow`), and the code must collect and retry at any allocation.

One run makes many refusals, where the knob makes one. A run near a vector
limit does the same: E9's small-limit step, `R_MAX_VSIZE=16M`, refuses
every request that would pass the limit, counting garbage. Some failures
below need more than one refusal in a run.

The two share the knob, so `robigo-rt` refuses to start with both set.

### What it finds on main

These are the conformance slice under the mode on this PR's base, main
7a628603, run locally with two jobs as the runner uses.

| run | time | passes | ratcheted cases that fail |
|---|---:|---:|---:|
| `ROBIGO_GC_MOVE=1` alone, for scale (on 78dcf1ee) | 10 s | 456/464 | 0 |
| `ROBIGO_RETRY_TORTURE=1` (the production, non-moving collector) | 174 s | 453/464 | 3 |
| `ROBIGO_GC_MOVE=1 ROBIGO_RETRY_TORTURE=1` | 1,144 s | 452/464 | 4 |

**The failing cases and their owners** (the foreman's routing):

| case | under the mode | with GC_MOVE too | owner |
|---|---|---|---|
| `tests/rt/conditions/onexit-unwind.R` | segfault | "unimplemented type 'unknown' in 'eval'" (a moved object read) | S1 |
| `tests/rt/capi/e8-unwind.R` | segfault | the same | S1 |
| `tests/r/lang/on-exit.R` | passes | the same | S1 |
| `tests/rt/conditions/trycatch-finally.R` | passes | the same | S1 |
| `tests/rt/parse/file.R` | segfault | passes | L |

**How many refusals each needs.** E9's own knob, one refusal per run, was
swept over every allocation of the three cases that crash under the mode
alone (35,666 runs, production collector):
- **`onexit-unwind.R`** differs at one point. `ROBIGO_FAIL_ALLOC_AT=1249`
  makes robigo-rt hang: it prints "on.exit caught: inner" and loops. This
  is on.exit code that catches its own error during an unwind, after one
  refused allocation. It goes to S1.
- **`e8-unwind.R`** (9,655 allocations) and **`parse/file.R`** (13,991)
  pass every single point: their crashes need two or more refusals in one
  run.

**What dominates the GC_MOVE run.** I timed each of the 456 ratcheted cases
alone under `ROBIGO_GC_MOVE=1 ROBIGO_RETRY_TORTURE=1`, six at a time: 2,392
CPU seconds in all.

| case | seconds |
|---|---:|
| `tests/rt/sort/radix-volume.R` | 548 |
| `tests/r/memory/finalizers-self-ref-envs.R` | 470 |
| `tests/r/jit/loop-vectors.R` | 150 |
| `tests/r/jit/loop-closed-form-super.R` | 94 |
| `tests/r/memory/auto-cycles.R` | 89 |
| `tests/r/jit/nested-arith-args.R` | 82 |
| `tests/rt/stress/finalizer-depths.R` | 62 |
| `tests/rt/capi/weak-keys-collections.R` | 50 |

Every other case takes 30 s or less.
- **radix-volume** is a `volume` case. The conformance runner leaves those
  out only under gctorture (`KNOWN_FEATURES`), so it runs under this mode.
- **finalizers-self-ref-envs** is the INTERIM(H8) case
  (`2026-11-17-gc-move-timeout.md`).
- **The two together** are 43% of the CPU time.

### When the CI steps come

Each comes as its own PR, as the foreman decided.
- **Without GC_MOVE, on rt pull requests:** about 3 minutes locally, once
  S1's and L's cases pass.
- **With GC_MOVE, on main only:** with `volume` left out, as gctorture's
  steps leave it out, the run drops by radix-volume's 548 s. The
  finalizers case is H8's. That leaves about 1,370 CPU seconds, or 11 to
  12 minutes on two jobs.

### Proof

- **Unit tests,** `crates/robigo-rtcore/tests/retry_torture.rs`. With the
  feature (`cargo test -p robigo-rtcore --features retry-torture`):
  - with the mode on, a build and a `retry_alloc` each run twice, with one
    `retry` collection each;
  - a build that allocates nothing runs once, and leaves the next
    allocation unrefused. Without the disarm, that test fails;
  - with the mode off, a build runs once.

  Without the feature, which CI's build is, turning the mode on panics
  with the build command.
- **Miri:**
  - CI's command, `cargo +nightly-2026-11-02 miri test --locked -p
    robigo-rtcore`, passes all 14 test binaries (313 s), the feature's
    refusal included;
  - with `--features retry-torture`, `nothing_else_is_refused` passes. The
    other test needs a collection, which Miri cannot run.
- **No cost without the feature:** `perf.py rt-bars` against main's binary
  at 7a628603: every row within ±0.05%, 0 failures.
- **The feature build** reproduces `onexit-unwind.R`'s segfault under the
  mode. A default build refuses `ROBIGO_RETRY_TORTURE=1` with the build
  command.
- **The two knobs together:** `ROBIGO_RETRY_TORTURE=1
  ROBIGO_FAIL_ALLOC_AT=5 robigo-rt -e 1` stops with "robigo:
  ROBIGO_RETRY_TORTURE and ROBIGO_FAIL_ALLOC_AT both use E9's knob: set
  one".
- **#769 pre-fix (ca1798cc), with the prototype of this mode:**
  - under GC_MOVE, `registry.R` with `environmentName(ns)` segfaults in
    one run;
  - `search.R` and `registry.R` give their normal output.
- **Discarded:** a first, cruder prototype collected before every build
  attempt, even when the closure allocates nothing. A refusal cannot
  happen there, and it flagged 317 cases.
- **Data:** `~/.cache/robigo-research/e9-retry-torture/` (README.md), with
  the prototype runs on 78dcf1ee.
