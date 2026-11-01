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
`.robigo/bin/robigo` of the current worktree.

## Correctness

| check | command | what it compares |
|---|---|---|
| unit tests | `cargo test --release -p <crate>` | crate-level tests |
| parser oracle | `cargo test --release -p robigo-syntax` | parse trees against GNU R's `parse()` |
| parse corpus | `scripts/parse-corpus.py check` | parse trees and parse errors of the fastr/gnur-tests/gnur-examples corpora and GNU R's own sources against GNU R's dumps (`tests/parse/passing/` ratchets) |
| conformance | `cargo test --release -p robigo-cli --test conformance` | `tests/r/**/*.R`: stdout, stderr, exit status as bytes, under the interpreter, the VM and the VM with JIT |
| corpora | `scripts/corpus.py run NAME` | below |

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
```

A case passes only if stdout, stderr and the exit status all match GNU
R's (addresses like `<environment: 0x...>` are masked). `script` mode runs
a case like `Rscript --vanilla FILE`; `echo` mode like `R --vanilla -f
FILE`, the transcript R CMD check compares. Cases whose GNU R output
differs between two runs (time, randomness without a seed) are marked
unstable and not scored.

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

Package sources are downloaded into `~/.cache/robigo-cran` (`src/`,
`build/`, `lib/` for robigo, `lib-oracle/` for GNU R) and are never
committed.

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
- `scripts/idiom-vs-r.py` and `scripts/callgrind-diff.py` measure single
  idioms and where instructions go.

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
- Performance: CI compares every push with `main`'s last instruction
  counts.

## CI

`.github/workflows/ci.yml`, on every push and pull request:

1. **build:** `cargo fmt --check`, release build, all `cargo test`s
   (unit, parser oracle, conformance in all tiers).
2. **corpora:** each corpus (FastR in four shards) against its ratchet.
   Results are artifacts; the summary shows pass counts.
3. **cran:** the manifest `tests/cran/packages.tsv` — CRAN packages with
   compiled code — installed from source with robigo and compared with
   GNU R's recorded result (§ CRAN packages with compiled code).
4. **perf:** instruction counts of every benchmark; on pull requests
   compared with `main` (fails if slower by more than 0.5% or newly
   wrong); on `main` also GNU R's counts, saved as the next baseline.
5. **progress** (main only): appends the commit's numbers (instruction
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
   DIR` draws the site locally.

Every job runs in a `debian:trixie` container with Debian's
`r-base-core 4.5.0-3`: the GNU R the corpora were recorded with, on the
same C library (GNU R uses glibc's `sinpi` and friends when they exist, so
other builds of GNU R 4.5.0 can differ in the last bit). The host runner
is `ubuntu-24.04` unless the repository variable `ROBIGO_RUNNER` names
another (e.g. a self-hosted machine).
