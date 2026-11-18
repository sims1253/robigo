## 2026-11-18: CI throughput: what the queue spends its runners on, two fixes, three proposals

- **Why:** at 05:20 UTC the GitHub-hosted queue was about 3.5 h deep (45 `ci`
  runs queued for 30 open pull requests), and the queue was the limit on
  every merge that day. The foreman's queue item (A1).
- **Data:** every `ci` run created 2026-11-17 08:00 to 2026-11-18 06:44 UTC
  (323 runs: 248 pull_request, 75 push) and their jobs, from the Actions
  API. Runner-minutes are a job's `completed_at - started_at` on a runner,
  each execution once: a re-run's attempt carries successful jobs forward
  under new ids with the same runner and times (74 such copies, 370
  minutes, are left out).
  Scripts and raw data: `~/.cache/robigo-research/a1/ci-data/`
  (`analyze.py`, `stacked.py`, `blocked.py`).

### Where the minutes went

21,440 runner-minutes (about 23,900 billed, an estimate rounding each job
up to a minute): 14,132 on pull requests, 7,309 on main. 77 pull requests merged in the same 24 h.

| job | jobs | runner-min | median queue wait |
|---|---:|---:|---:|
| corpora (10 shards) | 1,237 | 6,091 | 34 min |
| rt | 209 | 3,925 | 12 |
| perf | 125 | 3,330 | 25 |
| miri | 264 | 1,513 | 12 |
| test | 130 | 1,050 | 30 |
| base-subset | 230 | 859 | 12 |
| build | 152 | 628 | 20 |
| release-tests | 128 | 593 | 25 |
| bytecode-fidelity | 228 | 485 | 14 |
| perf-rt | 200 | 455 | 13 |
| perf-rt-build | 227 | 417 | 14 |
| fuzz | 128 | 393 | 31 |
| lint | 229 | 386 | 13 |
| the other 10 | | 1,316 | |

A job waited a median of 13 min (p90 45, max 115) for a runner on a pull
request, and 44 min (p90 90, max 151) on main. A green pull request run took
a median of 76 min (p90 157) from creation to `Check`; a green main run, 156
min (p90 260).

### 1. Superseded runs blocked their successors (fixed here)

`cancel-in-progress` did cancel a superseded run. But `Check` had
`if: always()`, and `always()` runs a job in a cancelled run too. So the
cancelled run queued `Check` for a runner, stayed `queued`, and kept the
concurrency group's one running slot; the new head's run stayed `pending`
behind it until `Check` got a runner.
Example: #877's 37d1c2fdd, whose 21 other jobs were all cancelled at
06:32:20 while `Check` waited.

- 60 pull request runs sat pending behind an older run of the same PR:
  1,268 minutes in all, median 17, max 58.
- At 06:44, 14 runs were pending, the longest for 71 minutes.
- 107 cancelled or queued runs had their `Check` waiting for a runner
  (median 12 min, max 66).

**Fix:** `Check` is `if: ${{ !cancelled() }}`. It still runs after failed
and skipped jobs, so a failure still fails it; it no longer runs in a
cancelled run, which now ends at once and frees the group.

**The price:** a skipped required check counts as passing, and a run
cancelled by hand on a pull request's head now leaves `Check` skipped. So a
head run must never be cancelled by hand (docs/workstreams.md says so); a
push cancels the superseded run by itself. The foreman's merge tooling
could also refuse a pull request whose head run's conclusion is not
`success`. Main is not affected: its runs have a group per commit and are
never cancelled by concurrency.

A tiny first job that cancels older runs through the API would not help:
the new run is `pending` at the workflow level, so none of its jobs starts
before the old run ends. A separate workflow's job would wait for a runner
in the same 3.5 h queue.

### 2. Stacked pull requests (fixed here, as approved)

20 runs were of stacked pull requests (base not main, or before their
retarget), 689 runner-minutes, 503 of them in jobs a stacked pull request
now skips. `select` sets `stacked` from `GITHUB_BASE_REF`. A stacked pull
request runs select, lint and rt (with rdb-data) only, plus agent-context
(its own workflow). It skips build, release-tests, test, counters, corpora,
fuzz, cran, smoke, perf, package-gate, miri, bytecode-fidelity,
base-subset, perf-rt-build, perf-rt and rt-corpora.

**Its `Check` fails, on purpose:** retargeting a pull request starts no run
(`pull_request` fires on opened, synchronize and reopened). A green stacked
`Check` would let it merge into main with the heavy jobs never run.
After a retarget, push, or close and reopen the pull request.

The `edited` event is not a way out: it fires on every title or body edit,
and with the workflow's concurrency group each such run would cancel the
running one.

### 3. The perf job's wait for main's baseline (proposal)

Measured over the 73 pull request perf jobs that ran:
- "Fetch baseline from main" (scripts/perf-baseline.py) held a runner
  for 1,924 minutes in all: 9% of all runner-minutes, and more than every
  other perf step together.
- The median wait was 23 min, p90 51.
- 31 of the 73 reached the 50-minute cap: 30 then fell back to an
  ancestor's numbers anyway, 1 was cancelled at 49 minutes. 48 waited more
  than 5 minutes.
- The measurement itself took a median of 7.3 min.

The wait is long because main's runs are the slowest in the queue: their
jobs wait 44 min for a runner.

Proposal, best first:
- **(a) Measure the base in the same job.** callgrind counts are
  deterministic, so the job can build robigo from the merge commit's first
  parent (HEAD^1, already fetched) and measure both binaries on the
  filtered benchmarks.
  - Costs one more build (`build` takes a median 4 min) and one more
    filtered measurement (7 min).
  - Saves the 23-min median wait and the 50-min tail.
  - Compares exactly the commit the merge is built on: no ancestor
    fallback, no coupling to main's queue.
  - Net: the 1,924 minutes of waiting go, and about 73 × 11 ≈ 800 minutes
    of work come, so about -1,100 runner-minutes a day, and a perf verdict
    that no longer depends on main's run.
  - Main's runs keep publishing `perf-main` for the progress history.
- (b) Cap the wait at a few minutes and fall back to the nearest ancestor
  with numbers (it already does after 50 min). This is cheap. The PR is
  then credited or blamed for main's own changes since that ancestor, the
  error the current design exists to avoid.
- (c) Measure first, upload the numbers, and judge in a job triggered by
  main's run (`workflow_run`). That job is not the PR's required `Check`,
  so it needs a commit-status write and the ruleset to require it. More
  moving parts than (a).

### 4. Main moved under a green pull request (proposal)

The ruleset does not require branches to be up to date, and a run tests the
merge with main as it was when the run started. Of the 74 merged pull
requests that had a green run in the window:
- 70 merged after main had moved past their last green run (median 4 main
  commits in between, max 35);
- 51 (69%) had, in between, a main commit touching one of the same crates.

That is how #853, #875 and #877 would have broken main without a textual
conflict.
(`staleness.py`: a run's creation time stands for the main commit it
tested, since GitHub makes the test merge when the run is triggered;
crates are compared by their `crates/<name>` directory.)

- **A merge queue (recommended).**
  - Each queued merge is tested as main plus the entries ahead of it, so a
    signature change on main is caught before the merge.
  - Per queue entry, with the jobs the foreman named (select, lint, build,
    rdb-data, rt with its rt-ci conformance, plus builtin-diff-data once
    #863 is in): about 0.3 + 1.7 + 4.1 + 0.5 + 20.5 (+ 3) ≈ 27-30
    runner-minutes. The wall time is the select → rdb-data → rt chain, about
    22 min plus the runner wait.
  - At 77 merges a day, about 2,100-2,300 runner-minutes (+10%), plus the
    rebuilds after a failure: each entry behind a failed one is rebuilt
    without it. The queue's merge limits (group sizes) only merge several
    entries at once; every entry is still built on its own, so they save no
    minutes. Its build concurrency (entries built at once, e.g. 5) keeps
    the wall time per merge near one build, about 22 min plus the runner
    wait.
  - perf, perf-rt, corpora, test and the rest stay judged on the pull
    request.
  - **What it needs:**
    - `merge_group:` in ci.yml's `on:`;
    - a `merge_group` mode in ci-select.py that selects that subset;
    - ci-check.py accepting those skips under `merge_group`;
    - concurrency groups that do not cancel queue runs;
    - the ruleset's "Require merge queue" with `Check` required;
    - the foreman's merge.sh adding to the queue (`gh pr merge --auto`)
      instead of merging.
- **A cheap "stale base" required check: not recommended.** Main's push
  would have to recompute a commit status on every open pull request (a
  required job cannot re-evaluate itself when main moves). With 51 of 74
  merges stale on a shared crate, it would force about 50 full pull request
  reruns a day (a pull request run averages 57 runner-minutes, about 2,900
  in all), each of which can go stale again while it waits. That costs
  more than the queue and protects less.
- "Require branches to be up to date": the same as above, with GitHub doing
  the forcing. Worse at this merge rate.

### 5. Docs-only pull requests

`select` gives `light` for docs/ and ledger-only diffs. That skips every
heavy job and the rt family, but it still runs lint, miri (5.4 min median)
and bytecode-fidelity (2.1 min), which cannot change for docs. Proposal:
`light` skips miri and bytecode-fidelity too (lint stays for its
workflow/markdown checks). This is small, about 8 runner-minutes per
docs-only push.

Found on the way: #863's paths (`tests/builtin-diff/**`,
`scripts/builtin-diff*`) match no rule in scripts/ci-selection.toml, so a
known-list regeneration runs the full matrix. It belongs in the `rt` rule;
it goes into #887.

### 6. Main's runs that end `cancelled` with every gate green

Of the 75 main runs, 35 ended `cancelled`. Some were cancelled by hand. In
34 of them, the `progress` job was cancelled while waiting: its job-level
group `progress` keeps one running and one pending job, and GitHub drops an
older pending job when a newer one arrives, even with
`cancel-in-progress: false`. So those commits' history lines are written
later by `history.py sync`, not by their own run. perf-baseline.py reads
the `perf-main` artifact, not the run's conclusion, so pull requests are not
affected. No change proposed; noted so that nobody reads those runs as
failures.
