# test infrastructure ledger

## 2026-11-08 (PR #503 review round 1): fix CI selection classification, package-gate inputs, and stale canary cache

- **why:** A coordinator/GPT-6.1 Sol review found that the first cut of CI selection
  could skip jobs that read the changed files or skip the package gate on changes
  that affect it, and that the installed canary could be reused from a stale cache.
- **fix (measured by the classification, not by runner time in this session):**
  1. `tests/**` is `full` (runtime=false): test scripts, corpora, parse cases, ratchet
     markers and expected outputs feed `test`/`corpora`/`parse`/`cran`/`smoke`, so a
     PR that only adds a wrong marker or expectation must not skip those jobs.
     `tests/packages/**` is `full` with `runtime=true` (markers, expected outputs,
     canary, selection, `rv.lock`) so the package gate runs. Only files no CI job
     reads stay `light`: `docs/**`, top-level `*.md`, `LICENSE*`, `.gitignore`,
     `.githooks/**`. The `runtime` rule (`crates/**`, `rlib/**`, `Cargo.*`,
     `rust-toolchain*`) is first so it wins over `*.md` for `crates/**/*.md` and
     `rlib/**/*.md`.
  2. Runtime-producing scripts and package-gate inputs are `runtime=true` before the
     generic `scripts/**`/`.github/**` rules: `build-natives.sh`, `natives-layout.h`,
     `robigo-home.sh`, `build.sh`, `build-rdbflat*.{sh,py,R}` produce the runtime/
     natives/R_HOME/rdbflat the gate runs in; `package-suite.py`,
     `test-package-suite.py`, `.github/actions/package-suite/**`, `.github/actions/setup/**`,
     `.github/workflows/ci.yml`, `.github/workflows/packages.yml` define the gate and
     must run it. Other `scripts/`/`benchmarks/`/`.github/` remain `full`,
     `runtime=false` (gates-and-infra).
  3. `scripts/package-suite.py` hashes the complete canary source tree (every file's
     path and contents) into `canary_library()`'s install path, and `ci.yml`/
     `packages.yml` include `tests/packages/canary/**` in the `robigo-packages` cache
     key, so editing `R/canary.R`, `NAMESPACE`, `DESCRIPTION`, tests or snapshots
     cannot validate against a stale installed canary.
- **verification:** `scripts/test-ci-select.py` (21 tests) and
  `scripts/test-package-suite.py` (65 tests) pass in this worktree. Dry runs:
  `tests/r/passing/strings/escape.R.pass` -> `full, tests, runtime=false`;
  `tests/packages/passing/rlang/a/b.pass` -> `full, package-gate-inputs, runtime=true`
  (package gate runs); `tests/cran/expected.json` -> `full, tests, runtime=false`;
  `scripts/build-natives.sh` -> `full, package-gate-inputs, runtime=true`;
  `scripts/package-suite.py` and `.github/actions/package-suite/action.yml` ->
  `full, package-gate-inputs, runtime=true`; `crates/x/README.md` -> `full, runtime,
  runtime=true`; `docs/foo.md` -> `light, no-binary-effect`. `cargo fmt --check` and
  `scripts/lint.sh` run before push; CI results recorded in the PR description.

## 2026-11-08: CI job selection, a package-regression gate, and robust package-suite caching

- **why:** A PR's CI ran every build-dependent job (~18 min wall, ~63 runner-minutes;
  baseline run 42518223579: build 5.3 min, perf 12.2 min critical path) even when the
  diff could not change the binary. The package suite ran only nightly and the nightly
  had been red on infrastructure (every job failed with `curl: (22) ... 504` downloading
  the `rv` release in run 42466532418) and on unnoticed runtime regressions (60 blocks in
  cli/rlang/Rcpp/htmltools around 2026-11-06/07; fixed on another track).
- **metric (before, PR CI run 42518223579, 2026-11-08):** wall 17m36s, 62.6 runner-minutes
  over 21 jobs (skipped: perf-diagnostic, progress). Critical path build 5.3 min + perf
  12.2 min. A docs-only PR ran the same heavy jobs (perf skipped only its own measurement
  via `perf-select.py`, after a full build). After: measured from the CI runs of this PR
  and a throwaway docs-only PR (recorded in the PR description).
- **fix:**
  1. `scripts/ci-select.py` + `scripts/ci-selection.toml` (`scripts/test-ci-select.py`):
     PRs whose every changed path cannot affect the binary (docs, ledgers, test data) skip
     the build-dependent heavy jobs; `crates/`, `rlib/`, `Cargo.*`, `rust-toolchain*` mark
     runtime changes; `scripts/`, `benchmarks/`, `.github/` run every heavy job
     conservatively but do not mark runtime. `main`/dispatch and any unmatched or unreadable
     diff run everything (fail-safe). `ci.yml` gates `build`/`test`/`corpora`/`fuzz`/
     `cran`/`smoke`/`perf` on `select.heavy`; `lint`/`miri`/`bytecode-fidelity` still run.
     `progress` keeps its dependencies and still records everything on `main`.
  2. `scripts/package-suite.py run --ratchet-only` (`ratchet_files`, `scripts/test-package-suite.py`):
     runs only packages and test files with `tests/packages/passing/` markers and fails on
     REGRESSED markers only (the canary still validates the run). `ci.yml` job `package-gate`
     runs it on runtime-changing PRs in 2 shards, in parallel with perf, with the pinned
     library restored from cache. Transitional: while `main` still carries the pre-existing
     cli/rlang/Rcpp/htmltools regressions fixed on another track, a PR based on that `main`
     can report those same regressions; the gate is correct once `main` is green.
  3. `.github/actions/package-suite/action.yml` retries the `rv` download with backoff and
     reuses an installed `rv`; `.github/workflows/packages.yml` and `ci.yml` cache
     `/usr/local/bin/rv` and `~/.cache/robigo-packages` keyed by
     `rv.lock`/`rproject.toml`/`sources.tsv`/`apt.txt`. This is the class of failure that
     produced the 504 flake in run 42466532418.
- **rejected:** gating the package suite on the whole suite per PR (hours). Making the
  package gate non-blocking by default (it would not be a gate); instead it is correct by
  construction and the pre-existing-regression transition is documented.
- **verification:** `scripts/test-ci-select.py` (19), `scripts/test-package-suite.py` (63),
  `scripts/test-perf-select.py` (29) pass; `cargo fmt --check` and `scripts/lint.sh` pass.
  Selection dry runs: docs-only -> `light, heavy=false`; runtime crate -> `full, heavy=true,
  runtime=true`; gate script -> `full, heavy=true, runtime=false`. CI run data for the
  "after" wall/runner-minutes and the throwaway regression-catching proof are in the PR
  description.

## 2026-10-31: kill 19-minute paired confirmation by measuring like-for-like at source and re-pairing only flagged regressions

- **why:** In CI, `Compare with main` running `perf-paired.py` took ~19m18s because
  it re-measured *all* 84 benchmarks for *both* binaries with cache off (336 callgrind runs).
  The historical root cause was that `perf-main` on main was measured with `rdbflat`
  cache ON while PRs ran with cache OFF (to avoid phantom +2.69% deltas on `nbody_2`),
  so candidate and baseline were not comparable at source.
- **metric:** `Compare with main` step duration: 19m18s -> ~3 seconds on clean runs.
  Total `perf` job duration: ~25m30s -> ~2m15s.
- **fix:**
  1. Main's perf run measures with the same cache state as PR runs (`export ROBIGO_RDBFLAT=""`),
     making `perf-main` and PR measurements directly comparable at source.
  2. The `rdbflat` win on `nbody_2` is reported on main in a separate diagnostic step
     (`--diagnostic-only`), never polluting the gate's numbers.
  3. `perf-paired.py` restricts paired confirmation to flagged regressions only:
     cases where candidate-vs-baseline delta exceeds 0.5%, candidate has wrong output,
     or candidate measurement failed. Unflagged cases keep the direct comparison.
  4. Flagged cases retain identical strictness: measured with exact binaries in isolated
     tempdirs with cache off (min 2 runs) against GNU R oracle or baseline output,
     failing the gate if the regression persists.
- **verification:**
  - 24 unit tests in `scripts/test-perf-paired.py` pass.
  - End-to-end proof: a deliberate +0.6% regression on `shootout/nbody_2` is flagged,
    re-paired, and fails the gate (`SLOWER shootout/nbody_2 +0.60%`, exit 1).
  - End-to-end proof: a candidate with wrong output is flagged, re-paired, and fails
    with `RuntimeError: candidate output differs from GNU R`.
  - End-to-end proof: previously red case verdict (`UNSUPPORTED`) is unchanged.

## 2026-10-31: cache GNU R oracle output in perf-main to measure robigo only in PRs

- **why:** `perf` job in CI took ~28m22s (e.g. run 41153399779: measure robigo
  & GNU R 8m00s, compare with main 19m18s). PR runs were executing `--runtime both`,
  measuring GNU R under callgrind across all 84 benchmarks just to establish
  that robigo printed the identical answer to GNU R.
- **metric:** PR `perf` step duration: eliminating GNU R callgrind measurement
  in PR runs cuts ~8m of unnecessary callgrind execution without weakening the gate.
- **fix:**
  1. `scripts/perf.py` keeps GNU R's `out` (and status) in `perf.json`,
     popping only `robigo`'s `out`. Main's `perf-main` artifact becomes a
     complete oracle record.
  2. `scripts/perf.py run` accepts `--oracle FILE`: loads baseline's GNU R counts,
     status, and stdout, and checks robigo's stdout/status against it. A differing
     output is marked `wrong: true`.
  3. `scripts/perf.py compare` recomputes `wrong` if candidate robigo output is
     present against baseline's recorded GNU R output/status.
  4. `.github/workflows/ci.yml` reorders the steps: fetches baseline from main
     before measuring, then measures robigo with `--oracle base/perf.json`
     (falling back to `--runtime both` if baseline is missing), then compares.
- **verification:** 19 unit tests in `scripts/test-perf-paired.py` verify oracle
  loading, output comparison, wrong marking on mismatch, and compare recomputation.

## 2026-10-30: conflict-free ratchets (one marker file per case)

- **why:** `tests/r/passing.txt` and `tests/corpora/*/passing.txt` were
  single append-only lists. Every PR that ratcheted a case edited the same
  lines, so every merge conflicted every other open PR (eight mechanical
  rebases in one night), and the "never remove an entry" rule forced the
  resolution by hand each time.
- **metric:** shared mutable lines in the ratchets, 32,226 entries in 8
  files (`tests/r` 273; capi 19, cran-purer 31, fastr 31,078, gnur-demos
  14, gnur-examples 780, gnur-tests 23, r-intro 8) -> 0: 32,226 empty
  marker files, one per case, each of which a branch adds (never edits).
  The required set is identical (proved below), the failure semantics are
  the same (a marked case that fails fails the run), and a marker that
  names no case is now a hard error instead of a silent no-op.
- **representation:** `tests/r/passing/<path relative to tests/r>.pass`
  (id `tests/r/strings/escape_origin.R` ->
  `tests/r/passing/strings/escape_origin.R.pass`) and
  `tests/corpora/NAME/passing/<case id>.pass` (an id with `/` nests
  directories). The `.pass` suffix keeps the markers out of every
  `*.R` glob (`collect()` in the conformance test, `find tests/r -name
  '*.R'` in `scripts/regen-expected.sh`), and makes a stray file in the
  directory detectable.
- **rejected:**
  - `merge=union` in `.gitattributes` for `passing.txt`: it only helps
    line appends, silently produces duplicate lines, does not catch a
    dangling entry, and leaves the file as the single append point
    (the brief's property is structural, not textual).
  - batched fragment files (`passing/<hash of the added ids>.txt`): fewer
    files, and different additions still get different files, but a hand
    edit of an existing fragment reintroduces the shared line, and a
    fragment name no longer says which case it ratchets.
  - an inverted "known-failing" list for fastr (99% of its cases pass):
    changes the semantics (a newly imported failing case would fail CI)
    and is not the same ratchet.
- **loader:** `crates/robigo-cli/tests/conformance.rs` (`load_ratchet`,
  ids are repository-relative as the old lines were) and
  `scripts/corpus.py` (`load_passing`, `add_passing`). Both still read a
  legacy `passing.txt` (the file the markers replaced) so branches that
  predate this change keep working; `scripts/ratchet.py migrate`
  converts it and removes it. `ROBIGO_CONF_UPDATE=1` and
  `scripts/corpus.py run --update` / `absorb` create markers; they never
  rewrite an existing one.
- **migration (one commit):** `scripts/ratchet.py migrate`, after
  `--dry-run`, printed the same set for every ratchet:
  `tests/r` 273 entries -> 273 markers, capi 19 -> 19, cran-purer 31 ->
  31, fastr 31,078 -> 31,078, gnur-demos 14 -> 14, gnur-examples 780 ->
  780, gnur-tests 23 -> 23, r-intro 8 -> 8 (`same set`, every legacy file
  removed). `scripts/ratchet.py check`: `total: 32226 ratcheted case(s)`,
  exit 0.
- **tests added:** Rust `ratchet_reads_markers`,
  `ratchet_rejects_marker_for_missing_script`, `ratchet_rejects_stray_file`,
  `ratchet_reads_legacy_list` (the dangling case is the old silent
  failure); Python `RatchetTests` (one marker per case, dangling marker,
  stray file, legacy list, `--update`, `absorb`) and the profile test now
  asserts the marker survives and no `passing.txt` is written.
- **verification:** conformance 274/275 before and after, same single
  pre-existing unratcheted pass (`tests/r/lang/closure_source_print_utf8.R`),
  0 regressions (a filtered `ROBIGO_CONF_UPDATE=1` run created exactly that
  marker; removed again so the migration set stays identical). A filtered
  update is now safe: it only adds files. `corpus.py run fastr --filter
  'TestBuiltin_paste'`: 60/60, 0 new, 0 regressions (the ratchet was read:
  an empty one would have reported 60 new). `corpus.py run r-intro`:
  5/13, the same 3 regressions main's harness reports on this host without
  `ROBIGO_NATIVES` (`devholdflush` unimplemented), unrelated to this
  change. `corpus.py absorb` on a results file with one unratcheted pass
  created its marker (removed again); `python3 scripts/test_corpus.py`
  12/12; `cargo fmt --check`.
- **conflict proof:** two branches that each add a marker for a different
  case add different paths, so `git merge` has nothing to reconcile;
  merging this branch into a branch with a legacy `passing.txt` is a
  delete/modify, resolved by keeping the deletion and running
  `scripts/ratchet.py migrate` (which reads the file and creates the
  markers), or by keeping the file: the loaders read both.
- **follow-up (done on `track/test-infra-parse-ratchet`):** the parse
  ratchet (`tests/parse/passing/<corpus>.txt`, 41,067 entries in 6 files,
  rewritten sorted by `--update`) had the same conflict shape.
  `scripts/parse-corpus.py` now uses `tests/parse/passing/<corpus>/<id>.pass`
  with the same loader/add/migrate semantics (`check` reports a marker
  that names no source as `RATCHET: marker names no source`, and
  `migrate` converts a legacy list after proving no entry is lost).
  Counts: rlib 834, rtests 95, fastr 31,374, gnur-tests 59,
  gnur-examples 1,153, cran 7,552 — every set identical; `check`:
  fastr 31,374/31,374, gnur-tests 59/59, gnur-examples 1,153/1,153,
  rlib 834/834, rtests 95/95, 0 lost.
- The corpus ratchets' per-PR diffs now list files instead of lines;
  `scripts/corpus.py absorb` creates a few hundred files for a big
  fastr shard, which is the intended shape.

## CRAN gate time limits (#200)

- A C API bug can present as a hang, and neither `scripts/cran-gate.py`
  nor the `cran` job had a limit, so a hang lasted GitHub's 6 hours.
- Measured: the gate step takes 47-61 s on CI for fastmap and digest,
  compiling included. Limits: 600 s per test step, 900 s per install,
  overridable through `ROBIGO_CRAN_{TEST,INSTALL}_TIMEOUT`.
  `timeout-minutes: 60` sits above the worst case of every step hanging,
  so the per-step outcome is what reports.
- Measured: `systemd-run --user --scope` execs the command, keeping PID,
  PGID and SID, so `killpg` reaches the tree under the memory cap too.
- Sol found the test failing in CI's kind of container: without an init
  nothing reaps the orphan, and `os.kill(pid, 0)` finds a zombie. The test
  reads the state from `/proc/PID/stat`. Reproduced in the oracle image.
- Proved with a hanging fake robigo and a real CRAN fetch: `fastmap 1.2.0:
  TIMEOUT (install after 2s)`, exit 1, no survivors.
