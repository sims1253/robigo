## 2026-11-17: the rt job's capacity (its 30-minute timeout)

- Why: the `rt` job hit `timeout-minutes: 30` on #798 (job 134096030525),
  #800 (134098057301) and #794 (134103751452). Each was cut inside the
  conformance step under `ROBIGO_GC_MOVE=1` with gctorture, 947, 960 and
  932 s into it.
- Where rt runs: GitHub's hosted runners only (`ubuntu-24.04`, 4 vCPUs).
  Self-hosted routing was removed on 2026-11-07 (5302c6fe): the privileged
  docker-in-docker made a container escape root on the host. The
  self-hosted slots were idle, so busy slots were not the cause, and routing
  rt back to them is not an option.
- Durations over the ~30 PR runs before H8: 17-29 min. The runner decides
  more than the cases do:

  | runner | release tests | conformance under `GC_MOVE=1` with gctorture |
  |---|---:|---:|
  | fast | 325-380 s | 420-560 s |
  | slow | 470-550 s | 700-960 s |

  The ratcheted cases grew from 456 to 514 in two days, and the collector
  steps grow with them.
- H8 (#797) made that step `ROBIGO_GC_MOVE=young:16`. On H8's own run
  (a fast runner) it took 142 s, and the job 13.7 min.
- What (this entry's PR):
  - robigo-conformance: `ROBIGO_CONF_JOBS=n` sets how many cases run at
    once. The default stays 2, for a `cargo test` that shares the machine
    with other test binaries.
  - The cases named in `ROBIGO_CONF_TIMEOUTS` start first. The workers take
    cases from the end of the list, so a slow case no longer starts late
    and holds the run up. This matters in the nightly's `--full-move` run,
    where `finalizers-self-ref-envs.R` gets 1200 s.
  - `scripts/rt-ci.py` gives every conformance step, and the nightly's
    run, a case per CPU. These steps run one test alone (`--exact`).
- Measured on 4 pinned CPUs on this box, with the steps' CI environments
  (wall time):

  | step | 2 jobs | 4 jobs |
  |---|---:|---:|
  | conformance under gctorture | 130 s | 93 s (-28%) |
  | conformance under `GC_MOVE=young:16` with gctorture | 198 s | 156 s (-21%) |
  | before H8: under `GC_MOVE=1` with gctorture | 754 s | 613 s; 547 s with slow cases first |

  CPU time stays the same (259 and 268 s; 396 and 413 s), so the extra
  workers add no work. Every run passes the same 507 of 514. Under
  `young:16`, one case (`finalizers-self-ref-envs.R`, 141 s alone in H8's
  ledger) bounds the step. Before this change, the plain run passed three
  times with 24 workers, and every other mode passed with 4, with the same
  results.
- The slowest runner after H8, estimated from the slow runs' steps:
  - release tests 550 s and conformance under gctorture 150 s;
  - under `young:16` about 255 s: 142 s times 1.79, the slow/fast ratio of
    the gctorture step;
  - the other steps 300 s, and setup 60 s.

  That is about 22 min with 2 jobs and about 20 min with a case per CPU.
- Timeout: 30 min keeps 1.5x that, so the number stays. The retry-torture
  step (E9, A1's next PR) adds a feature build and a run, and it moves the
  number with its own measurements.
- Rejected: routing rt to the self-hosted runners (security, above), and a
  separate job for the collector conformance steps. That job needs a second
  release build of 6-9 min for what is now about 6 min of steps.
