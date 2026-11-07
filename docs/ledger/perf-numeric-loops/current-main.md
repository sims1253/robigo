# Numeric loops on current main

Baseline: `2fb18b44c593dbec3efe3d00a412d184a8d23536`, release build,
GNU R 4.5.0, Callgrind 3.24.0. Command:

```sh
PATH="$HOME/.cache/robigo-research/R-4.5.0/bin:$PATH" \
LD_LIBRARY_PATH="$HOME/.cache/robigo-research/R-4.5.0/lib/R/lib" \
scripts/perf.py run --metric instr \
  --filter 'awf/Mandelbrot|mandelbrot_|convolution' --jobs 1
```

| Benchmark | Main instructions | GNU R instructions |
|---|---:|---:|
| rbench/awf/Mandelbrot | 201,173,604 | 463,911,482 |
| rbench/real/convolution | 258,117,353 | 1,362,933,727 |
| rbench/real/convolution_slow | 320,310,926 | 1,647,283,414 |
| rbench/real/convolution_v | 272,354,322 | 707,759,895 |
| rbench/shootout/mandelbrot_ascii | 416,952,493 | 719,028,855 |
| rbench/shootout/mandelbrot_naive_ascii | 734,783,586 | 1,024,455,249 |
| rbench/shootout/mandelbrot_noout | 491,000,365 | 813,274,029 |
| rbench/shootout/mandelbrot_noout_naive | 410,313,531 | 782,423,276 |

All eight outputs match the oracle. The awf fixture prints `verified: FALSE`
in both runtimes because its pinned verification data omits parameter 14.

## Stack audit

- #332: main already has nested native regions, heat accumulated across loop
  entries (`vm::loop_heat`), int-or-double `JType::N` values, and integer index
  handling. `compile::seq_length_form` recognizes `seq(length=...)`, but the JIT
  still lacks `ForSeqLength` support for a range created inside its region. There is no `Op::Bitwise`.
- #385: main already records unsupported calls as `JitLoop::leaves`, counts
  native back edges, and resumes in the VM. `JType` has no complex variant;
  scalar complex arithmetic therefore cannot run in native loops. The old
  split-variable representation cannot be brought across unchanged.
- #397: guards currently call `guard_ok` individually on every native entry.
  Calls now allow fresh closure instances with shared code, free variables,
  and captured cells. Caching the old stack's closure pointer would discard
  those semantics. Experiment with grouping only loop-environment guards;
  keep call targets, argument operators, and callee operators checked.
- #403: integer arithmetic and conversions still emit NA tests. `JType::N`
  already guarantees non-NA integer values when its tag is integer; a general
  analysis must distinguish that representation from possibly-NA `JType::I`.

The seven stack regression scripts were regenerated with GNU R 4.5.0,
without copying recorded output or adding ratchet markers prematurely.

## Experiments on this base

Grouped loop guards alone measured 201,053,454 instructions for awf/Mandelbrot
(-0.060%) and 258,125,768 for convolution (+0.003%). Keep the narrow cache;
callee targets and their guards remain independent because main supports fresh
closure instances, captured cells, and free variables. The key includes the
search environment's id as well as its epoch and up to three local frame ids
and function-binding versions. Deeper chains retain the individual checks.

`loop-guard-chain.R` exposed stale VM guards after replacing an existing
operator in a locked enclosing frame. `GuardState` now includes that frame's
version in revalidation. The test covers insertion/removal at depths zero
through four and replacement in a locked frame, and agrees in all three tiers.

The imported `loop-leave.R` exposed an existing condition-message difference.
R 4.5.0 `src/main/eval.c:asLogicalNoNA` uses "argument is not interpretable as
logical" for a numeric NA. Ported that rule; the inline review of #385 claimed
the opposite, so the C source and a fresh 4.5.0 recording settled it.

The #403 experiment tracks integer/logical NA facts in the existing native
region graph. It guards entry assumptions explicitly, kills facts for written
slots, intersects at joins, distinguishes the exhausted `ForStep` edge, and
returns no facts if convergence exceeds the work budget. Integer overflow
still leaves to the VM before assigning the result. An NA entry value retries
compilation without its former assumption. `JType::N` remains main's tagged
int-or-double representation and is excluded from these integer facts.

Facts are deliberately not carried into inlined callees or whole closures in
this experiment. Main's call trees now include nested argument expressions,
captured cells, and free variables. Reusing the stale pass unchanged would
miss those writes. The existing native call handling stays conservative.

Quick-build verification: all eight restored/new loop scripts match GNU R
4.5.0 byte-for-byte in interpreter, VM, and VM+JIT; runtime tests pass (99 unit,
1 bytecode/lazy-load, 2 lazyload). JIT traces confirm native compilation and overflow
exits in `loop-nonna.R`. Release measurements follow below.

A trace of main's `mandelbrot_naive_ascii` rejects its loop at the complex
`Const` at op 44. Both convolution loops compile (ops 30 and 38). Complex
scalars remain a measured gap, rather than a claim that the old 15x win was
reproduced. The native vector kernels from the earlier numeric-loops work
already exist on main. This PR keeps the independent guard/NA work; it does
not transplant the old stack's split types, entry counters, or complex ABI.

The first release candidate reduced convolution from 258,117,353 to
251,745,038 instructions (-2.47%) and convolution_slow from 320,310,926 to
308,426,505 (-3.71%). All eight target outputs matched GNU R. It was rejected
because robigo/closures increased from 886,202,710 to 899,869,133 (+1.54%).
GuardState initialization was calling frame_of_locked even for a frame whose
parent was already on the search path. The next candidate avoids that check
for shallow frames; locked enclosing frames retain the version check.

FastR arithmetic/comparison/loop/bitwise/complex slice: 831/832, zero ratchet
regressions. The existing failure is
`library/base/TestSimpleArithmetic/f7fa6afb895ab94a`.
Full release conformance: all 461 scripts agree across interpreter, VM, and
VM+JIT. Oracle conformance is 447/461, with exactly one ratchet failure:
`graphics/cairo-devices.R`, because this machine lacks the grDevices native
`bmVersion`. The other failures are existing unratcheted gaps. Both lint
passes completed successfully. The final constructor optimization is being
rebuilt and remeasured; final validation is recorded below.

The shallow-frame optimization still left closures at 891,719,643 (+0.623%).
Packing the extra version into GuardState's existing tuple padding did not
resolve it (891,926,708, +0.646%). Paired Callgrind profiles measured
886,209,624 on main and 891,912,900 on the branch: 5.45M extra instructions
were in the two VM dispatch bodies, only 0.25M in GuardState::revalidate.
Per-line costs increased across guards, scalar reads, assignments, and calls.

Rejected per-frame parent-version tracking entirely. Restore GuardState to
main verbatim. Instead, invalidate the existing epoch on the rare replacement
of a function-like binding in a locked environment (Env::bump_fn_version).
Every guard cache already depends on this epoch; ordinary lookups gain no
additional work, and this also invalidates memos through deeper locked chains.
This is a minimal shared-file change in env.rs.


## Final proof

All 48 selected programs match GNU R 4.5.0. The instruction gate reports
4 faster, 0 slower above 0.5%, 0 broken. The worst increase is
rbench/simple/profiler-rsa, +0.2013%. Full counts, including every rbench/simple
and robigo program, are in [current-main.csv](current-main.csv).

| Benchmark | Main instructions | Branch instructions | Change |
|---|---:|---:|---:|
| rbench/awf/Mandelbrot | 201,173,604 | 201,157,074 | -0.008% |
| rbench/real/convolution | 258,117,353 | 251,087,966 | -2.723% |
| rbench/real/convolution_slow | 320,310,926 | 307,753,693 | -3.920% |
| rbench/real/convolution_v | 272,354,322 | 272,403,683 | +0.018% |
| rbench/shootout/mandelbrot_ascii | 416,952,493 | 417,008,941 | +0.014% |
| rbench/shootout/mandelbrot_naive_ascii | 734,783,586 | 734,822,992 | +0.005% |
| rbench/shootout/mandelbrot_noout | 491,000,365 | 491,044,915 | +0.009% |
| rbench/shootout/mandelbrot_noout_naive | 410,313,531 | 410,376,516 | +0.015% |
| robigo/fannkuch | 646,679,615 | 605,186,373 | -6.416% |
| robigo/mandelbrot | 232,766,275 | 227,669,519 | -2.190% |

Commands after the release build:

```sh
# Both files combine the target set and rbench/simple/ plus robigo/.
scripts/perf.py run --metric instr --runtime robigo \
  --filter 'awf/Mandelbrot|mandelbrot_|convolution|rbench/simple/|robigo/' \
  --oracle .robigo/perf/numeric-main-all.json \
  --out .robigo/perf/numeric-after-all.json --jobs 2
scripts/perf.py compare .robigo/perf/numeric-main-all.json \
  .robigo/perf/numeric-after-all.json --threshold 0.5
```

The PATH and LD_LIBRARY_PATH above select the GNU R 4.5.0 installation for
both runs. The after run reuses its recorded oracle outputs and counts.

Final checks on the epoch-invalidation design:

- cargo fmt --check and both passes of scripts/lint.sh pass.
- cargo test --profile quick -p robigo-runtime passes: 99 unit tests and
  3 integration tests.
- R_HOME from the GNU R 4.5.0 installation, ROBIGO_CONF_BIN set to the release
  binary, cargo test --profile quick -p robigo-cli --test conformance:
  447/461 oracle matches; all 461 agree across the three tiers. Eight of nine
  harness tests pass. The only ratchet failure is cairo-devices.R, with the
  expected missing grDevices bmVersion native on this machine.
- Expanded FastR arithmetic/comparison/loop/bitwise/complex and environment,
  eval, assignment and binding-lock slice: 1105/1106, zero regressions.
- scripts/ratchet.py check passes. Eight new script markers were added;
  none were removed.

The final change keeps the independently proven #403 integer analysis and
#397 loop-guard grouping in main's design. It preserves main's heat, tagged
numbers, nested regions, index handling and call escape/re-entry. Native
bitwise operations, native ForSeqLength inside a region, complex scalar
registers and interprocedural NA facts remain open. The measured complex
benchmark baseline and the unsupported complex Const trace establish that
remaining opportunity; this PR does not claim the stale stack's 15x result.
