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
| conformance | `cargo test --release -p robigo-cli --test conformance` | `tests/r/**/*.R`: stdout, stderr, exit status as bytes, under the interpreter, the VM and the VM with JIT |
| corpora | `scripts/corpus.py run NAME` | below |

### Corpora

`tests/corpora/` holds large collections of R code with GNU R's recorded
output (`scripts/corpus.py`, manifest `tests/corpora/corpora.toml`):

| corpus | cases | mode | source |
|---|---|---|---|
| `fastr` | 33,562 | script | FastR's unit-test snippets (GPL-3), one R expression each |
| `gnur-tests` | 59 | echo | GNU R's own `tests/*.R` |
| `gnur-examples` | 1,153 | echo | the examples of every help page of base R's packages |

```sh
scripts/corpus.py run fastr --filter 'builtins/TestBuiltin_paste/'   # a slice
scripts/corpus.py show fastr builtins/TestBuiltin_paste/0aefed6423ab410d
scripts/corpus.py run gnur-tests --update   # also record new passes
scripts/corpus.py score
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

- `tests/r/passing.txt` and `tests/corpora/*/passing.txt` list cases that
  must keep passing. A failing listed case is a regression and fails CI.
  Never remove entries to make a run pass; fix the regression.
- New passes are added with `--update` (locally) or `scripts/corpus.py
  absorb` on CI's result files (artifacts `corpus-*`).
- Performance: CI compares every push with `main`'s last instruction
  counts.

## CI

`.github/workflows/ci.yml`, on every push and pull request:

1. **build:** `cargo fmt --check`, release build, all `cargo test`s
   (unit, parser oracle, conformance in all tiers).
2. **corpora:** each corpus (FastR in four shards) against its ratchet.
   Results are artifacts; the summary shows pass counts.
3. **perf:** instruction counts of every benchmark; on pull requests
   compared with `main` (fails if slower by more than 0.5% or newly
   wrong); on `main` also GNU R's counts, saved as the next baseline.

The runner is `ubuntu-24.04` unless the repository variable
`ROBIGO_RUNNER` names another (e.g. a self-hosted machine).
