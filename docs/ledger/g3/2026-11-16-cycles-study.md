## 2026-11-16: do estimated cycles predict wall time better than instructions? A study for the lead

The lead's decision (c) (`2026-11-16-m1-exit-lead-decisions.md`): "The
instruction gate was only meant as a proxy to improve walltime on a number
of different hardware platforms. If number of cycles would be the better
number, we should at least look at it."

This entry measures callgrind's estimated cycles against instructions, as
predictors of wall time, on three CPUs, and makes a recommendation. It
changes no gate.

**Recommendation:**
- **The gate:** keep instructions; do not gate on estimated cycles.
- **Report-only:** add estimated cycles to `perf-rt` as a report-only
  column, under a pinned cache model.
- **Wall time:** neither count can stand in for wall time at the
  few-percent level. Wall time has to be measured, at milestones, on fixed
  hardware.

The numbers follow.

### The question in numbers

- **Estimated cycles** are `scripts/perf.py`'s model: Ir + 10·D1m +
  100·LLm, from callgrind with `--cache-sim=yes --branch-sim=yes`
  (`scripts/idiom-vs-r.py --cycles`). Instructions are Ir alone.
- **A predictor is judged** on how well its change predicts a change in
  wall time. The gate's question is whether a pull request makes a row
  slower.

### The data

- **13 binaries of robigo-rt,** giving 11 changes:
  - **10 consecutive main merges** that changed robigo-rt between the M1
    exit evidence's two runs: #718, #717, #719, #725, #722, #728, #726,
    #727, #730 and #731. They run from `2671f453` to `c536597d`, each
    against the main commit before it. #721 is left out: it added a test
    only.
  - **D5's recycling** (#702, `7849ccf9` against `4169b271`). This is the
    change where the two counts disagreed (`p1-reruns.md:186-192`).
- **The rows,** from `benchmarks/rt/calls-rows.toml`: `plain`, `defaults`,
  `named`, `env-fn`, `superassign`, `s3`, `fib`, `binarytrees`,
  `e7-paste`, `e7-match` and `e7-unique`.
  - All 11 run with inlining on.
  - The first 8 also run with `ROBIGO_NO_INLINE=1`, for real calls, where
    D5 was judged.
- **Counts:** `scripts/rt-bars.sh` with the e7 rows (callgrind, N against
  2N), twice per binary and mode, on the desktop. They were also measured
  under fixed cache models through `VALGRIND_OPTS` (below), and natively on
  m0pad and on the CI runner.
- **Wall time** follows `docs/g3.md` §13's protocol
  (`data/…/study-wall.py`).
  - **A sample:** one round's (T(2K) − T(K)) / K. It uses the same loop
    script that `idiom-vs-r.py` counts, so startup and setup cancel. K is
    sized so that a K-iteration loop takes about 0.3 s, so every sample is
    far above 0.05 s.
  - **The rounds:** 11, interleaved over every (binary, row), with the
    order rotated each round and the order of K and 2K alternated.
  - **Medians** are compared.
- **The machines:**

  | machine | CPU | where | changes |
  |---|---|---|---|
  | desktop | AMD Ryzen 9 5900X (Zen 3) | WSL2 Linux 6.18, glibc 2.44; other agents built on it during the run (load average 2-9) | 11 |
  | m0pad | AMD Ryzen 5 PRO 5650U (Zen 3, mobile) | CachyOS, glibc 2.44, idle | 11 |
  | CI runner | AMD EPYC 9V74 (Zen 4), 4 vCPUs | GitHub's `ubuntu-24.04`, in `perf-rt`'s Rust image (Debian trixie, glibc 2.41); its own builds | 3: #702, #722, #731 |

  - **The CI run** is a one-off workflow, `data/…/cycles-study.yml`, on a
    branch of its own that was never merged and is now deleted (run
    43934601040).
  - **A change of wall time counts as real** when it is larger than both
    twice its noise and the gate's 0.5%. The noise is the standard error
    of the two medians, from their samples' spread.

### Results

Each cell compares the two predictors as instructions / cycles. Cycles are
under the pinned model (32 MB, 16-way last-level cache); the CI runner's own
model gives the same changes, within 0.02 points.

| | desktop | m0pad | CI runner |
|---|---|---|---|
| points (change × row × mode) | 209 | 209 | 57 |
| median noise of a change | 3.1% | 1.3% | 1.1% |
| real wall changes (of them slower) | 40 (16) | 83 (31) | 25 (6) |
| wall's spread where neither count moved 0.1% | 5.6% (75 points) | 4.0% (75) | 3.3% (17) |
| correlation with the wall change, r | 0.14 / 0.12 | 0.51 / 0.54 | 0.26 / 0.36 |
| direction right, on real changes | 72% / 75% | 67% / 71% | 68% / 88% |
| direction right where a count moved ≥ 1% | 50 / 55 of 75 | 57 / 64 of 75 | 18 / 23 of 26 |
| a 0.5% gate: real regressions caught | 2 / 2 of 16 | 2 / 4 of 31 | 0 / 1 of 6 |
| a 0.5% gate: false alarms | 2 / 12 | 2 / 10 | 0 / 2 |
| spread of ns per unit across rows and binaries, inlining on / off (CV) | 0.49 / 0.38; 0.13 / 0.10 | 0.40 / 0.31; 0.15 / 0.11 | 0.39 / 0.30; 0.19 / 0.15 |

**What the table says:**

1. **Neither count predicts a change's wall time to the gate's
   precision.**
   - Wall time moves with a spread of 3.3% to 5.6% where neither
     instructions nor cycles move at all. That is several times the
     measurement noise on m0pad and the runner (1.1-1.3%).
   - So most of it is not noise. It is what callgrind does not model,
     most likely code and data layout: alignment, branch prediction, the
     µop cache.
   - At 0.5%, both gates miss nearly every real wall regression: 27 to 29
     of 31 on m0pad.
2. **Cycles get the direction right somewhat more often** than
   instructions, where a count moves. The clearest case is D5's
   recycling, the change that started the question:

   | binarytrees, #702 | instructions | cycles (pinned) | wall |
   |---|---:|---:|---:|
   | desktop | +0.21% | −2.72% | −8.2% |
   | m0pad | +0.21% | −2.21% | −9.3% |
   | CI runner | +0.21% | −2.61% | −9.4% |

   - With inlining off, `named` on the CI runner gave −5.1% in cycles and
     −5.9% in wall time, against +0.46% in instructions.
   - Cycles also track absolute cost better: their spread of ns per unit
     is about a quarter lower.
3. **Cycles are worse as a gate.** At 0.5% they raise five to six times
   the false alarms instructions do, and catch at most two more real
   regressions.
   - Cycles move with simulated cache conflicts that wall time does not
     show.
   - #722's cut to fib is an example where both counts are wrong: −8.6%
     in both, but −0.5% to −0.9% in wall time on m0pad and the runner.

### Cycles depend on the host

- **The cache model:**
  - Callgrind takes its cache sizes from the CPU it runs on. It picked
    a 64 MB last-level cache on the desktop, 16 MB on m0pad and 256 MB on
    the runner.
  - It picked a direct-mapped (1-way) cache on all three, which is no
    real cache.
  - So `perf.py`'s cycles for one binary depend on the runner. Across
    the four models measured, a change's cycles move by up to 2.0 points:
    the SD is 0.22 to 0.27 points against the pinned model.
- **Across machines,** with the same binary and glibc 2.44 on m0pad and
  the desktop:
  - **Instructions** differ by up to 0.52%, on `s3` and `e7-unique`.
  - **Cycles** differ by up to 2.7% with the same direct-mapped model,
    and up to 0.80% with the pinned 16-way one.
  - The cause is not established: likely the two kernels' address layouts
    and glibc's cache-size tunables.
  - On one machine, two runs agree within 0.016% for instructions and
    0.017% for cycles.

  So a cycles gate would also have to be paired on one runner, as
  `perf-rt` already is, with the cache model pinned.
- **For the M1 bars:** instructions also move between machines, by up to
  half a percent. The bars compare robigo-rt in CI with GNU R numbers from
  one machine. `perf-rt`'s comparison with main is paired, so it is not
  affected.

### Cost

- **Callgrind's time:** callgrind with the cache and branch simulation
  takes 2.2 times as long. The 11 rows on one binary, in sequence, take
  49.7 s against 22.6 s.
- **In `perf-rt`:** its callgrind steps are most of its gate step (36 s on
  the demonstration run, for two binaries). That puts the column at about
  +30 to 40 s on a job that finishes far ahead of `rt`.

### Recommendation

- **Gate:** keep the paired instruction gate as it is. Do not gate on
  estimated cycles.
  - They are no better at catching wall regressions.
  - They raise five times the false alarms.
  - They need a pinned cache model and pairing to be stable.
- **Report-only:** add estimated cycles under a pinned cache model to
  `perf-rt`'s table, with no failure. The model would be
  `--I1=32768,8,64 --D1=32768,8,64 --LL=33554432,16,64`.
  - **Why:** cycles show the changes in memory behaviour that instructions
    miss (D5's), and they cost about 35 s, off the critical path.
  - **With it:** the runner's CPU should be printed (exit evidence, open
    items).
  - **Not filed here.** It is a `perf-rt` change, and is ready to make
    when the lead has read this entry.
- **Wall time:** if wall time on several platforms is the goal, it has to
  be measured. Neither count can protect it at the few-percent level,
  because layout moves it by about 4% on its own. The tool for that is
  §13's interleaved medians:
  - at each milestone, or nightly;
  - on fixed hardware of at least two CPU families (today: this desktop,
    m0pad, and a CI runner class);
  - with each change compared within a machine.

  A per-PR wall-time gate on shared runners would gate on the layout
  floor.

### Caveats

- **The changes:** 11 changes on small rows, all of them robigo-rt's own
  recent work. The runner covers only 3.
- **The desktop** was loaded by other agents, which made its noise 3.1%;
  m0pad and the runner were quiet.
- **The CPU families:** two, Zen 3 and Zen 4, both AMD. No Intel CPU was
  measured.
- **The layout floor** might be reduced with code alignment or the PGO
  build. Neither was tried.

### Raw data

`data/2026-11-16-cycles-study/`:
- the counts per model: `cg-desktop-host.json`, `cg-pinned.json`,
  `cg-m0padmodel.json` and `cg-cimodel.json`, measured on the desktop;
- the native counts: `cg-m0pad-native.json` and `cg-ci.json`;
- the wall times: `wall-{desk,m0pad,ci}-{on,off}.json`;
- the analyses: `an-*.json` and `final.json`;
- the scripts as run: `study-*.py`, `study-cg*.sh` and
  `cycles-study.yml`.
