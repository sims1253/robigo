# test infrastructure ledger

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
