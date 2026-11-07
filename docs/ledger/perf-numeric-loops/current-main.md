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
  handling. `compile::seq_length_form` recognizes `seq(length=...)`; native
  treatment must be assessed separately. There is no `Op::Bitwise`.
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
1 host oracle, 2 lazyload). JIT traces confirm native compilation and overflow
exits in `loop-nonna.R`. Release measurements follow below.

A trace of main's `mandelbrot_naive_ascii` rejects its loop at the complex
`Const` at op 44. Both convolution loops compile (ops 30 and 38). Complex
scalars remain a measured gap, rather than a claim that the old 15x win was
reproduced. The native vector kernels from the earlier numeric-loops work
already exist on main. This PR keeps the independent guard/NA work; it does
not transplant the old stack's split types, entry counters, or complex ABI.
