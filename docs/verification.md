# robigo: verification

Everything robigo does is checked against GNU R 4.5.0 by a machine. This
document lists the checks, how to run them, and the rules that keep
results from sliding back.

## Build

```sh
scripts/build.sh          # release build, copied to .robigo/bin/robigo
```

All worktrees share cargo's target directory (`~/.cache/cargo-target`),
so its `release/robigo` belongs to whoever built last. `scripts/build.sh`
builds and copies under a lock; the harnesses below use
`.robigo/bin/robigo` of the current worktree. A workspace crate's
artifacts have the same name in every checkout, so each checkout names
itself to its crates (`ROBIGO_WORKTREE` in `.cargo/config.toml`) and
cargo rebuilds them when another checkout built them last.

## Correctness

| check | command | what it compares |
|---|---|---|
| unit tests | `cargo test --release -p <crate>` | crate-level tests |
| parser oracle | `cargo test --release -p robigo-syntax` | parse trees against GNU R's `parse()` |
| parse corpus | `scripts/parse-corpus.py check` | parse trees and parse errors of the fastr/gnur-tests/gnur-examples corpora and GNU R's own sources against GNU R's dumps (`tests/parse/passing/` ratchets) |
| conformance | `cargo test --release -p robigo-cli --test conformance` | `tests/r/**/*.R`: stdout, stderr, exit status as bytes, under the interpreter, the VM and the VM with JIT |
| corpora | `scripts/corpus.py run NAME` | below |
| fuzzing | `scripts/fuzz.py parse\|rds` | mutated scripts through `parse()` and mutated RDS files through `readRDS()`: a panic, failed allocation, stack overflow or fatal signal is a finding; reproducers land in `.robigo/fuzz/` |
| builtin coverage | `scripts/builtin-coverage.py --check BIN` | the names.c builtins robigo lacks or registers as the wrong kind, and the "not implemented yet" / "not supported" refusals in the sources, against `scripts/builtin-coverage.baseline`: one that is not listed fails (`--update` lists it, and the review sees the diff) |

Everything beyond the gate — valgrind's other tools (memcheck, massif),
sampling (`scripts/samply.sh`), the cargo-fuzz harnesses, differential
eval fuzzing (`scripts/fuzz-eval.py`: against GNU R, `--tiers` for the
interpreter, the VM and the JIT, `--emi` for dead code), test-case reduction
(`scripts/reduce.py`) and compiler-output inspection — is documented in
`docs/profiling.md`, with
the local-machine setup and this round's findings in
`docs/ledger/perf-tools.md`.

### Timezone fixtures

A conformance script can have a `.tzdir` sidecar, for example
`tests/r/datetime/zones.tzdir`. It contains one directory path relative to
the script. `scripts/regen-expected.sh` and the conformance runner resolve
it to an absolute path and set `TZDIR` before starting GNU R or robigo,
including interpreter, VM and JIT runs. Both clear inherited `TZDIR` for
scripts without a sidecar. A missing fixture directory fails the run.

Named-zone tests use `tests/r/datetime/zoneinfo`, pinned to Debian tzdata
2026e. Its README documents provenance and regeneration. Record with
Debian's GNU R 4.5.0, never the host's newer R. The fixture keeps timezone
rule updates from changing expected output or masking runtime regressions.

### Corpora

`tests/corpora/` holds large collections of R code with GNU R's recorded
output (`scripts/corpus.py`, manifest `tests/corpora/corpora.toml`):

| corpus | cases | mode | source |
|---|---|---|---|
| `fastr` | 31,374 | script | FastR's unit-test snippets (GPL-3), one R expression each |
| `gnur-tests` | 59 | echo | GNU R's own `tests/*.R` |
| `gnur-examples` | 1,153 | echo | the examples of every help page of base R's packages |
| `gnur-demos` | 16 | echo | every `demo()` of GNU R's base packages, as `demo(..., echo = TRUE)` runs it |
| `r-intro` | 13 | echo | the runnable code of "An Introduction to R", one script per chapter |
| `cran-purer` | 41 | script | pure-R CRAN examples, demos and standalone tests from five pinned packages |
| `capi` | 20 | script | C cases compiled by each case's `build.sh` (`R CMD SHLIB`), driven from R; GNU R's output is `robigo-capi`'s (G2) target, weighted coverage measured by `scripts/capi-census.py coverage` |

```sh
scripts/corpus.py run fastr --filter 'builtins/TestBuiltin_paste/'   # a slice
scripts/corpus.py show fastr builtins/TestBuiltin_paste/0aefed6423ab410d
scripts/corpus.py run gnur-tests --update   # also record new passes
scripts/corpus.py score
scripts/triage.py gnur-demos                # first difference per failing case
scripts/reduce.py --corpus gnur-demos --case ID   # shrink one (triage prints the command)
```

A case passes only if stdout, stderr and the exit status all match GNU
R's (addresses like `<environment: 0x...>` are masked). `script` mode runs
a case like `Rscript --vanilla FILE`; `echo` mode like `R --vanilla -f
FILE`, the transcript R CMD check compares. Cases whose GNU R output
differs between two runs (time, randomness without a seed) are marked
unstable and not scored.

`--tier vm` runs the cases in the bytecode VM without the JIT (the top
tier on wasm32) against the same expected output and ratchet, so a VM
or JIT change that a corpus case can tell apart fails there; CI runs
every corpus this way too. It extends `compiled_matches_interpreted`
(`tests/r` only) to the corpora. `--tier interp` (the AST interpreter
alone) is a diagnostic, not a gate: GNU R's own AST interpreter differs
from its bytecode in a few places robigo reproduces (the `value =`
argument of a complex assignment's error call, what `trace()` prints),
and GNU R never runs base's code in it. On 2026-11-08 the VM tier
matched the default on every case of every corpus measured (all but
`capi` and `cran-purer`); the interpreter differed on four ratcheted
cases, all of that kind. Tier runs write `NAME.TIER.json` and never
add markers.

```sh
scripts/corpus.py run fastr --tier vm --filter 'TestSimpleVectors'
scripts/corpus.py show fastr library/base/TestSimpleVectors/0b6df291325c0b6c --tier interp
```

Run `scripts/corpus.py record NAME` (needs GNU R 4.5.0) after adding or
changing cases; `scripts/corpus.py import NAME` re-imports a corpus from
upstream.

### CRAN packages with compiled code

`scripts/cran-gate.py` installs real CRAN packages from source into
robigo — `robigo CMD INSTALL`, robigo's `R_HOME`, headers and
`libR.so` — and runs their own test files (`docs/core.md` §8). The
manifest `tests/cran/packages.tsv` is the ratchet: one line per package
(name, version, its tests directory), and `tests/cran/expected.json`
holds GNU R 4.5.0's result for each, recorded in the oracle image.

```sh
scripts/cran-gate.py run                     # install + test, compare
scripts/cran-gate.py run fastmap --diff      # one package, differing expectations
scripts/cran-gate.py try glue@1.8.1          # explore where a package stops
scripts/cran-gate.py record                  # GNU R in robigo-oracle:4.5.0
```

The tests run through `crates/robigo-capi/gate/shim.R`, a base-R-only
reimplementation of the tinytest/testthat API that counts expectations
and prints `PKG VERSION: N expectations passed, M failed`. It must
behave the same under both implementations — it is plain R, so it runs
unchanged in the oracle image; `record` is its only writer of
`expected.json` and it runs GNU R, never robigo. A package passes when
the counts (and, when anything fails, the failing expectations) match
GNU R byte for byte.

A C API bug can show up as a hang, so each install and test step has a time
limit (`ROBIGO_CRAN_INSTALL_TIMEOUT`, default 900 s;
`ROBIGO_CRAN_TEST_TIMEOUT`, default 600 s). A step over its limit has its
whole process group killed; the package's outcome is `timeout`, distinct
from a mismatch or a crash, and the run goes on to the next package.
`scripts/test-cran-gate.py` checks this.

Package sources are downloaded into `~/.cache/robigo-cran` (`src/`,
`build/`, `lib/` for robigo, `lib-oracle/` for GNU R) and are never
committed.

### Top CRAN packages

`scripts/package-suite.py` runs the own test suites of the most used CRAN
packages, the ones with over 900,000 downloads in 30 days and over 500
packages using them (20 at snapshot 2026-11-01), and scores robigo against
GNU R's outcome of every test block. The score and the per-package table are
on the progress page. `tests/packages/README.md` lists the files.

```sh
scripts/package-suite.py run                  # all of them, compare, ratchet
scripts/package-suite.py run magrittr R6      # some of them
scripts/package-suite.py show R6 --file 'test-finalizer'   # what differs, ranked
scripts/package-suite.py install              # the tree from source, with robigo
scripts/package-suite.py record [PKG...]      # GNU R, three times
scripts/package-suite.py smoke                # user scripts, transcript-checked
scripts/package-suite.py smoke --record       # GNU R: smoke expected transcripts
```

The smoke tier (`tests/packages/smoke/cases/*.R`) is the block score's
complement: whole user scripts — attach packages, do representative
work, print results — whose stdout, stderr and exit status must equal
GNU R 4.5.0's recorded transcript byte for byte (addresses and temp
paths masked) on the same pinned library. A script whose point is
that it errors passes by erroring identically; one that hangs where
GNU R exits is a `timeout`, never a pass. Header directives:
`# needs: pkg ...` makes a case `absent` (unscored) where a package
is not in the library — that is how the user's own GitHub-only
packages are carried until they are pinned — and `# timeout: N`.
`smoke --record` runs GNU R only and is the only writer of
`tests/packages/smoke/expected/`; `passing/` markers work as
everywhere else, an unmarked failure is a pending gap listed on the
progress page under "Package scripts as users run them". The tier
runs in minutes on every pull request (`.github/workflows/ci.yml`
job `smoke`), where the block suites are nightly: PRs get smoke plus
the conformance and corpus ratchets, the nightly keeps the full
blocks. Case budget: keep scripts small and tightly packed — a case
earns its CI minutes by forcing variable contents across the C
bridge (mask indices, loop counters) or by pinning state that must
survive across calls, not by exercising API breadth.

It runs in `robigo-oracle:4.5.0`, built on demand from
`tests/packages/Dockerfile`. It finds robigo in `.robigo/bin` with `$ROBIGO`
and the natives in `$ROBIGO_NATIVES`, as CI does, and it needs `.robigo/home`
built inside the image (`scripts/robigo-home.sh`).

What is measured:

- **Real frameworks.** The tests run under real testthat and tinytest, not the
  counting shim of `tests/cran`, so the numbers of the two are not comparable.
- **Same library.** `tests/packages/rv.lock` pins the 221 packages of the tree
  (the selection, its Suggests, their dependencies) at one PPM snapshot. GNU R
  and robigo load the same GNU-built binaries, so a difference can only come
  from the runtime. Whether those packages also build from source with robigo
  is the second number, `install`; rv does not build them, because a source
  install has to go through `robigo CMD INSTALL`.
- **One process per test file.** `tests/packages/runner.R` streams events as
  they happen and uses no package that is under test, and a supervisor kills a
  file at 3 GB or at `max(60 s, 20 × GNU R's time)`. A crash costs one file. The
  block running at the time is `interrupted` and the rest `notrun`.
- **No network, snapshots on.** Every worker runs with `NOT_CRAN=true`, `CI=true`,
  a fresh `$HOME` and only a loopback interface. `NOT_CRAN=true` matters: a
  CRAN-style run skips the snapshot tests, which are the ones that compare
  output exactly.
  A package that needs an HTTP test server (curl, `server` in
  `selection.tsv`) gets a pinned httpbin on that loopback, started per test
  file for GNU R and robigo alike (`tests/packages/README.md`, Test servers).

A block **matches** when its pass, fail, error, skip and warning counts and
its skip reasons all equal GNU R's. So an extra warning, a lost expectation
and a skip GNU R did not take are all differences.

- Scored: blocks GNU R runs clean in all three recordings.
- Not scored: blocks GNU R fails, skips, or does not repeat. A block GNU R
  skips that fails on robigo counts as `skip_diverged`.
- A package is **green** when every scored block matches, nothing diverged,
  every file finished, and at least 80% of its blocks are scored. The floor
  is there for what this machine cannot run (a missing Suggests, the
  network). A block GNU R skips steadily for a reason the package's own code
  always takes leaves the floor's denominator: `tests/packages/own-skips.tsv`
  lists those reasons by hand, each with the source line that makes it
  unconditional. Such a block is still a block, and robigo must skip it the
  same way or it diverges. httr is the case: 33 of its 96 blocks call
  `skip("httpbin is not reliable")` or `skip("Currently broken")`.

The headline is the share of scored blocks that match, across all packages,
with the green count beside it. A package at 0/20 green has no gradient; a block
share moves with every fix.

The canary is checked first: `tests/packages/canary` has one test per outcome,
including an unexported function through testthat's cloned namespace and a
snapshot both ways. If it fails, the run is invalid: it cannot update the
ratchet, it sets no score, and the page says so. A bug in how robigo runs
testthat must not look like thousands of failing packages.

`passing/` is a ratchet like the corpora's. A lost marker is retried once, so
a difference that does not repeat is reported as `flaky`, not as a regression.
A failing block is not a gap to record under protocol rule 8 by itself; the
cause it points to is (`docs/workstreams.md`).

The suites take hours, so they run nightly on the binary of main's newest
green CI run (`.github/workflows/packages.yml`, four shards) and do not gate
merges. The nightly installs `rv` and the pinned package library with retries
and caches them across runs (keyed on `tests/packages/rv.lock` and
`tests/packages/apt.txt`), so a transient download failure does not turn the
run red. A score is comparable only with scores from the same kind of machine.
Time limits scale with GNU R's recorded times, so a faster machine loses fewer
files to timeouts: the same main binary scores 5,222 blocks on 2 cores and
5,729 on 8. The nightly always runs on the same runner type, so its trend is
consistent; compare a local run with a local run. The result merges into that
commit's line of the history. A pull request that changes the binary runs the
ratcheted blocks only (`package-suite.py run --ratchet-only`, sharded) and
fails on REGRESSED markers; a pull request that touches the harness tries the
harness on magrittr (`packages.yml`). Examples and vignettes are not part of
this metric: examples can follow as a corpus, like `cran-purer`, and vignettes
mostly test pandoc and knitr.

## Performance

`scripts/perf.py` runs the benchmark suites in `benchmarks/benchmarks.toml`
(robigo's own `benchmarks/suite` and Ř's RBenchmarking: Are We Fast Yet,
the shootout, micro-benchmarks, real programs) on robigo and GNU R.

```sh
scripts/perf.py fetch                                   # external suites
scripts/perf.py run --metric instr --filter shootout/   # callgrind counts
scripts/perf.py run --metric time --runtime both        # wall time, idle machine
scripts/perf.py compare before.json after.json          # the gate
```

- **Instructions (callgrind) are the gate.** They are exact and
  repeatable, so a change is judged without wall-clock noise. A change may
  not make any benchmark more than 0.5% slower.
- **Wall time** against GNU R is the headline number, measured on an idle
  machine.
- **A wrong answer is not fast.** A program whose output differs from GNU
  R's is reported as wrong and not counted.
- **Pull requests run only the benchmarks their changed paths can
  affect** (see below); `main` runs all of them.
- **Creep on main.** The gate allows 0.5% per benchmark per pull request,
  so small regressions can add up. After each main run,
  `scripts/history.py check` compares robigo's counts with the median of
  the 7 main runs before it and with the newest run at least a week older
  (the anchor: counts barely vary between runs, so one old run is a fair
  reference, and it catches a creep whose steps each stay inside the
  band). A benchmark regressed when it is above by more than 1% or three
  times the spread its window shows (1.4826 × the median absolute
  deviation), the geometric mean by more than 0.5%; a benchmark whose GNU
  R count moved as much changed itself (its program or input) and is not
  compared. A regression the run before did not show opens an issue
  labelled `perf-regression` (or comments on the open one), with the pull
  request that raised each count most. Main stays green.
  Locally: `git fetch origin progress-data && git show
  origin/progress-data:history.jsonl > h.jsonl && scripts/history.py check
  h.jsonl [--commit SHA]`; `scripts/test-history.py` tests it.
- `scripts/idiom-vs-r.py` and `scripts/callgrind-diff.py` measure single
  idioms and where instructions go. GNU R is a rival, not the target:
  `idiom-vs-r.py --idioms` reports each idiom against its floor too, the
  instructions of a plain C version of the same work
  (`benchmarks/idioms.toml`), and `callgrind-diff.py --bench NAME --causes`
  splits a benchmark's instructions by cause (`benchmarks/causes.toml`).
  See `docs/profiling.md`, "Floors and causes".
- `scripts/rivals.py` compares wall time with every other R: GNU R, Ř and
  FastR, run in containers with the clock inside (`rivals.py setup` builds
  them). It reports one fresh process, startup included, and the fastest
  warmed-up batch; only batches of 50 ms or more count, so programs that
  take microseconds are still timed to about 2%. This is the finish line
  of "the fastest R", which instruction counts against GNU R cannot be.

### Path-scoped perf gate

The perf job is most of a pull request's CI time, and most pull requests
touch no hot code. `scripts/perf-select.py` maps the paths a pull request
changes to the benchmarks that exercise them; the mapping is data in
`benchmarks/selection.toml` (groups of benchmark-name regexes, and rules
from path globs to groups; the first rule matching a file wins).

- **Paths.** The diff of GitHub's test merge commit against its first
  parent, main's commit (`git diff HEAD^1 HEAD`; the job checks out with
  `fetch-depth: 2`). The same commit is the baseline
  (`scripts/perf-baseline.py`), so the compared set is exactly the PR's.
- **What runs.** `perf.py run --filter <union of the selected groups>`. The
  gate is unchanged: instruction counts, 0.5%, paired confirmation. The
  compare steps treat baseline benchmarks outside the run's recorded
  `selection` as not compared (one line, not `REMOVED`); a selected
  benchmark that vanished is still reported.
- **Fail-safe.** The full suite runs unless the event is `pull_request`, the
  diff could be listed and *every* changed file matches a rule. An unmatched
  path (a new directory, `value.rs`, `dispatch.rs`, `robigo-core`, ...), an
  unreadable diff, an empty diff or a malformed mapping runs everything.
  Build, toolchain, `rlib/`, `benchmarks/`, the perf scripts, `ci.yml`, the
  bytecode VM and its compiler (every closure runs on it) and the numerics
  core (`%%`, long double sums, the RNG) are explicit `all` rules. Only paths that cannot change an instruction
  count (docs, tests, the other workflows) select `none`: the job then
  records that in its summary, skips the baseline wait and the measurement,
  and passes.
- **`main`** (and `workflow_dispatch`) always runs the full suite and
  uploads `perf-main`, so every merge still records the full next baseline.
  Narrowed pull-request runs upload `perf`, which nothing consumes.
- **Dry run.** Nothing needs CI to see a decision:

  ```sh
  scripts/perf-select.py --list --changed crates/robigo-runtime/src/jit/closure.rs
  git diff --name-only origin/main... | scripts/perf-select.py --paths-from - --list
  scripts/perf-select.py --base origin/main --head HEAD       # your branch's diff
  ```

  `scripts/test-perf-select.py` (run by the job before selecting) pins
  realistic path sets (VM, builtins, parser, numerics, docs-only, mixed,
  unknown), checks that every group matches benchmarks, every group is used
  and every rule pattern matches a tracked file, and that the fail-safes
  hold.
- **Changing the mapping.** Add a path to a rule only when you can say which
  benchmarks exercise it; to add a benchmark to the set a path selects, add
  its name to a group. A new directory needs no entry: until it has one it
  runs everything. Weigh the cost by instruction counts: four programs
  (`knucleotide_brute*`, `flexclust_kcca`) are ~75% of the suite's
  callgrind time, so a rule that keeps them out saves far more than one
  that trims many small programs.

### Counter gate

A whole benchmark's instruction total can hide a path that got ten times
worse. `scripts/counters.py` counts the work a loop should not do and holds
it, per iteration, on fixed fixtures (`tests/counters/*.R`):

```sh
scripts/counters.py build      # quick profile, --features counters -> .robigo/bin/robigo-counters
scripts/counters.py check -v   # every fixture in the ast, vm and jit tiers
```

- **The counters** (`crates/robigo-runtime/src/counters.rs`): vector
  copies (copy-on-modify), promises, environments, allocator calls, VM ops
  (by kind too, as `op:<Op>`) and JIT loops, rejects and deopts. They exist
  only with `--features counters`; a release build has no code for them.
  The counted binary prints a `robigo-counters: {...}` line to stderr at exit.
- **Bounds** are header lines of each fixture, per iteration, in every tier
  or the ones named: `#| vector_copies <= 0`, `#| jit vm_ops <= 0`,
  `#| ast,vm allocs <= 2`. The gate runs each fixture at two loop lengths
  (`n` is defined for it) and divides the difference, so startup and
  warm-up cancel; counts are deterministic, so bounds are exact.
- **Known failures** (`tests/counters/known.toml`): a bound that fails today,
  the number measured and its cause. A listed bound may fail by no more than
  what is recorded; one listed that passes fails the run until it is taken
  out; one not listed that fails fails the run. The list only shrinks
  (`check --record` rewrites it, keeping causes; the diff is the review).
- **Adding a counter** is one line in `counters!` and a `count(...)` call;
  adding a check is a header line. CI's `counters` job runs
  `scripts/test-counters.py` and the gate.

## The ratchets

- `tests/r/passing/`, `tests/corpora/*/passing/` and
  `tests/parse/passing/<corpus>/` hold the ratchets: one
  empty marker file per case that must keep passing, named `<case>.pass`
  (`tests/r/passing/<path relative to tests/r>.pass`,
  `tests/corpora/NAME/passing/<case id>.pass`). One file per case means
  two branches that ratchet different cases add different files and never
  conflict; the single `passing.txt` lists they replaced conflicted on
  every addition. A marker that names no script or case fails the run
  (the old lists let a dangling entry do nothing, silently).
- A failing ratcheted case is a regression and fails CI. Never remove a
  marker to make a run pass; fix the regression.
- New passes are added with `--update` (locally) or `scripts/corpus.py
  absorb` on CI's result files (artifacts `corpus-*`); both create
  markers. `ROBIGO_CONF_UPDATE=1` marks conformance scripts.
- `scripts/ratchet.py check` lists the ratchets and fails if a marker
  names no case; `scripts/ratchet.py migrate` converts a legacy
  `passing.txt` (still read, for branches that predate the markers).
- Positive controls: a gate that cannot fail looks exactly like a clean
  run, so each checks first that it can. The conformance test fails when a
  marker of a script it selected matches no script that ran (ids spelled
  differently would require nothing). `scripts/corpus.py run` fails when a
  changed stdout compares equal or a ratcheted case's masks erase all of
  its stdout. `scripts/ratchet.py
  check` fails on an empty conformance ratchet, `scripts/parse-corpus.py
  check` on a ratchet it has no records to check, and
  `scripts/builtin-coverage.py --check` when it does not see `c` and
  `paste0` on both sides or extracts no refusal message.
- Performance: CI compares every pull request with `main`'s instruction
  counts for the commit it is built on (the benchmarks its paths select).

## Unsafe code

`docs/architecture.md` has the rules; the compiler and clippy enforce them
through `scripts/lint.sh`. `RString` and the `Rc` alias writes run under Miri:

```
rustup component add --toolchain nightly miri
cargo +nightly miri test -p robigo-runtime --lib -- value::
```

## CI

`.github/workflows/ci.yml`, on every push and pull request:

1. **select** (pull requests): `scripts/ci-select.py` maps the changed paths
   to the jobs to run (`scripts/ci-selection.toml`). Only files no CI job reads
   skip the build-dependent heavy jobs: docs, top-level `*.md`, repo metadata
   and hooks (markdown under `crates/` or `rlib/` counts as runtime, not docs).
   Test scripts, corpora, ratchet markers and expected outputs (`tests/**`)
   run every heavy job, because a wrong marker or expectation can pass CI and
   break `main`. Changes that can affect the binary's behaviour or the package
   regression gate's inputs also run the **package-gate**: `crates/`, `rlib/`,
   `Cargo.*`, `rust-toolchain*`; `tests/packages/**`; runtime-producing scripts
   (`scripts/build-natives.sh`, `scripts/natives-layout.h`,
   `scripts/robigo-home.sh`, `scripts/build.sh`, `scripts/build-rdbflat*.{sh,py,R}`);
   and the gate itself (`scripts/package-suite.py`, `scripts/test-package-suite.py`,
   `.github/actions/package-suite/**`, `.github/actions/setup/**`, `.github/workflows/ci.yml`,
   `.github/workflows/packages.yml`), plus the selection itself (`scripts/ci-select.py`,
   `scripts/ci-selection.toml`, `scripts/test-ci-select.py`). Other gate scripts, benchmarks and CI run
   every heavy job conservatively but do not run the package gate. G3's
   runtime crates (`robigo-rt`, `robigo-rtcore`, `robigo-rtbuiltins`, `robigo-rtapi`,
   and `robigo-heap` when it lands) and their cases
   (`tests/rt/**`) select `rt` mode: the robigo binary does not link them, so
   such a pull request runs lint, miri, bytecode-fidelity and the **rt** job
   and skips the heavy jobs and the package gate; the workspace manifests
   (`Cargo.toml`, `Cargo.lock`) still run everything. `main` and
   other events always run everything. Fail-safe: an unmatched path or an
   unreadable diff runs everything. `scripts/test-ci-select.py` checks the mapping.
   It also makes sure the CI image for this tree exists (below) and hands
   its references to the other jobs.
2. **lint:** `cargo fmt --check` and `scripts/lint.sh`.
   **miri:** `RString` and the `Rc` alias writes (§ Unsafe code).
   **bytecode-fidelity:** bytecode metadata and GNU bytecode fixtures. These
   jobs do not depend on the release binary and still run when a pull request
   skips the heavy jobs.
   **counters:** its own quick build with `--features counters` and the
   counter gate (§ Counter gate); a heavy job, skipped with the others.
   **rt:** G3's runtime crates (`scripts/rt-ci.py`): it checks that no other
   workspace member depends on them (the premise of `rt` mode; it fails at
   the flip, when the rule must go), then runs their `cargo test --release`
   and the same under `ROBIGO_GCTORTURE=1`. Every pull request but a
   docs-only one runs it; the modes of `docs/g3.md` §16 add steps here.
3. **build:** release build, base-package natives, the binary the heavy
   jobs use; the ratchet markers, the parse-corpus ratchet and the builtin
   coverage baseline (`scripts/builtin-coverage.py --check`) are checked
   against it.
4. **test:** all `cargo test`s (unit, parser oracle, conformance in all
   tiers). The harnesses build in the quick profile and conformance tests
   the release binary (`ROBIGO_CONF_BIN`).
5. **corpora:** each corpus against its ratchet, once as robigo runs by
   default and once with `--tier vm`: FastR in four shards, the six small
   corpora together in one job per tier (`group: small`; each alone spent
   as long on setup as on its cases). Results are artifacts; the summary
   shows pass counts. FastR's shards split by a hash of the case id: on
   2026-11-12 they held 7,754-7,919 cases each and the slowest 1% of cases
   took 2% of a shard's time, so a split by recorded duration would gain
   nothing (docs/ledger/test-infra.md).
6. **fuzz:** `scripts/fuzz.py` on the parser and on unserialize, one
   minute each with a fixed seed, and `scripts/test-reduce.py` (the
   reducer, the parse-tree mutator and EMI against GNU R 4.5.0).
7. **cran:** the manifest `tests/cran/packages.tsv` — CRAN packages with
   compiled code — installed from source with robigo and compared with
   GNU R's recorded result (§ CRAN packages with compiled code).
8. **smoke:** user-level package scripts
   (`scripts/package-suite.py smoke`), transcript-checked against GNU R on
   the pinned package library.
9. **package-gate** (pull requests that change the binary): every block the
   package ratchet already covers (`tests/packages/passing/`), sharded,
   with the pinned library and `rv` restored from cache
   (`package-suite.py run --ratchet-only`). It fails on REGRESSED markers
   only and runs in parallel with perf, so it must not lengthen the
   critical path. The full suites remain nightly
   (`.github/workflows/packages.yml`).
10. **perf:** instruction counts of every benchmark; on pull requests only
    the benchmarks the changed paths can affect (§ Path-scoped perf gate),
    compared with `main` (fails if slower by more than 0.5% or newly
    wrong); on `main` all of them, and also GNU R's counts, saved as the
    next baseline.
11. **progress** (main only): appends the commit's numbers (instruction
    counts of every benchmark, passing cases of every corpus and of the
    `tests/r` conformance scripts, the failing case ids of every corpus
    with at most a few hundred — the open gaps —, builtins implemented)
    as one line to `history.jsonl` on the orphan branch
    `progress-data`, then publishes the charts, badges and a page with
    every benchmark and corpus on GitHub Pages
    (https://sims1253.github.io/robigo/); the README embeds them. Nothing
    is committed to `main`, so the repository does not grow by an image
    per merge. `scripts/history.py sync` also records main runs the
    history lacks (a cancelled run, a failed push) from their artifacts,
    for as long as those exist; `scripts/history.py render HISTORY --out
    DIR` draws the site locally. Main always runs every job and still
    records everything it records today. Then it checks for slow perf
    creep (§ Performance) and opens or comments on a `perf-regression`
    issue; that never fails the run.
12. **Check:** the one check the `main` ruleset requires (with
    `agent-context`'s, below). `scripts/ci-check.py` passes when every job
    above passed or was skipped for a reason `select` gave, and fails on a
    failure, a cancel or any other skip (GitHub counts a skipped required
    check as passing). A new gating job goes into its `needs`;
    `scripts/test-ci-check.py` fails until it does.

Caches are saved by `main` only (rust-cache `save-if`,
`actions/cache/save`, `.github/actions/package-cache`); pull requests
restore them. A pull request's own entries are readable by that pull
request alone, and they had filled half of the 10 GB the repository gets,
evicting `main`'s.

Other workflows on pull requests: **agent-context** fails when `AGENTS.md`,
which every agent session reads in full, grows, unless the pull request has
the `agent-context-increase` label. **pr-delta** labels the pull request by
size (`size:XS` ... `size:XXL`, changed lines outside tests) and comments
its test delta: conformance scripts, ratchet markers per suite (removed
markers called out), Rust and Python tests (`scripts/pr-delta.py`).
**platforms** is an on-demand lane on macOS and arm64 Linux, which `ci.yml`
does not test: `gh workflow run platforms.yml --ref BRANCH -f
runner=macos-15 -f packages=robigo-runtime -f conformance=tests/r/strings`.

Every job runs in a `debian:trixie` container with Debian's `r-base-core
4.5.0-3`: the GNU R the corpora were recorded with, on the same C library
(GNU R uses glibc's `sinpi` and friends when they exist, so other builds
of GNU R 4.5.0 can differ in the last bit). The host runner is GitHub's
`ubuntu-24.04`. In `ci.yml` that container is the CI image,
`ghcr.io/sims1253/robigo-ci` (`.github/ci-image/Dockerfile`): `base` (R and
the harnesses' tools), `rust` (base and the pinned toolchain) and
`packages` (base and `tests/packages/apt.txt`). `scripts/ci-image.sh` tags
each by a hash of its inputs and `select` builds and pushes a missing tag,
so a pull request that changes an input tests its own image.
`.github/actions/setup` installs nothing inside it and stays the way the
other workflows set up a plain `debian:trixie`. Self-hosted runners were tried (#479) and removed: each
job needed its own Docker daemon, and `docker:dind` runs privileged, so a
job escaping it would be root on the host.
