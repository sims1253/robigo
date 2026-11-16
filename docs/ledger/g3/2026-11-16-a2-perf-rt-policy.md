## 2026-11-16: A2, perf-rt's CPU and baseline policy

The second item to settle before M2/M3 planning (`2026-11-16-m1-signoff.md`),
as the M2/M3 plan's §E.b words it. The lead adopted the cycles study's
recommendation (F3, `2026-11-16-cycles-study.md`). The instruction gate is
unchanged: 0.5% against main's binary of the merge base, measured on the
same runner, and the M1 bars.

### What changes

- **The runner's CPU is printed.** The job prints the CPU's model and a
  signature, `scripts/perf.py`'s `cpu_signature`. It hashes what glibc's
  CPU dispatch reads, and so what moves callgrind's counts:
  - vendor, family, model and stepping;
  - the model name;
  - the feature flags;
  - every cache's level, type, size, associativity and line size.
- **GNU R's side of every bar is measured on the gating runner's CPU.**
  - **Why:** the M1 bars compared robigo-rt, measured in CI, with GNU R's
    counts from one developer machine. glibc's dispatch moves GNU R's
    counts with the CPU, so a bar mixed two CPUs.
  - **Measuring it:** `perf.py rt-gnur` measures GNU R's rows (instructions
    and estimated cycles) and its peak RSS on this machine. It records the
    GNU R version, the CPU, its signature, the cache model and valgrind's
    version.
  - **Using it:** `rt-bars --gnur FILE` uses those numbers in place of the
    bars file's. It refuses a file measured for another GNU R than the rt
    pin, on another CPU signature, or under another cache model.
  - **Caching it:** the `perf-rt` job keeps the file in the Actions cache,
    keyed by (oracle image, CPU signature, valgrind, the rows file, the
    bars file, `idiom-vs-r.py`, `rss.py`). A runner class pays for it
    once. A miss measures it in the job.
- **Estimated cycles are reported, never failed on.** `rt-bars --cycles`
  takes them from the same callgrind runs as the instructions. The
  simulated caches are pinned: `--I1=32768,8,64 --D1=32768,8,64
  --LL=33554432,16,64` (`PINNED_CACHE`).
  - The table shows each row's cycles and their change against main.
  - Pinning keeps the counts from moving with the runner's detected caches.
    The study found up to 2 points of difference in a change's cycles
    otherwise.
- **Two jobs.** GNU R and robigo-rt have to be measured on one runner, so:
  - **`perf-rt-build`** (CI's Rust image, rt's cargo cache) builds the PR's
    binary and main's binary of the merge base, and uploads them;
  - **`perf-rt`** runs in the 4.6.2 oracle image, which has GNU R, a C
    toolchain and Python, and installs valgrind (Debian's, about 15 s). It
    measures both binaries, and GNU R on a cache miss.

  `Check` needs both, and `scripts/ci-check.py` allows their skip only
  when `rt` is skipped. A job in the Rust image could not run GNU R 4.6.2:
  that image's R is Debian's 4.5.0, and a container job cannot run docker.
- **The scripts:** `idiom-vs-r.py --rows --no-robigo` and `rss.py
  --no-robigo` measure GNU R alone.

### What stays

- **Today's robigo's numbers** are still the committed ones (G1), measured
  on one developer machine. They are E2's fib and binarytrees bars and E7's
  rows. The plan's policy names GNU R's side only. Measuring them per CPU
  signature too would mean building the old runtime at G1's commit in CI on
  a cache miss. That is a question for the foreman.
- **The bars file's [gnur] numbers** stay, as the record of the first
  baseline. A local `rt-bars` run without `--gnur` uses them.
- **The rival** is still the oracle image's hardened build. A0 measures what
  a stock build changes, and the lead's F2 makes that the rival.

### Proof

Measured on this desktop (AMD Ryzen 9 5900X, signature `1be0f69f7d2b7252`):
- **`rt-gnur`** ran inside `robigo-oracle:4.6.2` with Debian's valgrind
  3.24.0 in 38 s. Its six GNU R rows agree with the bars file's
  oracle-image numbers within 0.11%: s3 19,191.2 against 19,213.2.
- **`rt-bars --gnur … --cycles`** ran with main's binary as both binaries
  in 63 s, with 4 jobs.
  - Every row is within ±0.02% of main in instructions and cycles.
  - s3 is 0.977x against GNU R on this signature.
  - The table prints the CPU, its signature and the cycles columns.
- **Tests:**
  - `scripts/test-perf-rt-bars.py`: 20 tests. They cover the signature
    (stable across flag order; changed by a flag, a cache size or a model),
    the refusals (another pin, signature or cache model) and the bars using
    GNU R as measured.
  - `scripts/test-ci-check.py`: the build job's skip rule.
  - actionlint reports nothing new.
- **In CI:** this PR's own run, the first per-signature measurement on a
  GitHub runner. It shows whether GNU R's count there moves s3 across its
  bar: its margin was 0.9% against the developer machine's GNU R.
