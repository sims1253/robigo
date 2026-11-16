## 2026-11-17: A0b, the instruction bars' GNU R rival is GNU R's literal default build

### The lead's decision (Max, 2026-11-17)

F2, revised after A0 (`2026-11-16-a0-oracle-gap.md`). The instruction bars'
GNU R rival is GNU R's literal default configure: static libR, unhardened,
from the pinned 4.6.2 tarball, in CI's image toolchain, measured per CPU
signature inside `perf-rt` as A2 set up (`2026-11-16-a2-perf-rt-policy.md`).
It replaces the oracle image's own build, which has Debian's hardening
flags and a shared libR. E1's s3 bar becomes unmet and is owed by P3, M3's
bar-closing package.

A0 measured why the rival matters. Against the oracle image's build, GNU R's
literal default counts about 10% fewer instructions:
- 7% of that comes from linking libR statically;
- 2.5% comes from leaving out Debian's flags.

The toolchain stays CI's.

### What changes

- **The rival's build:** `scripts/gnur-stock.sh PREFIX` builds the pinned
  tarball (`scripts/r-src.sh`) with `./configure --prefix=PREFIX` and
  nothing else. It clears compiler and linker flags from the environment,
  so the build is configure's default: `-g -O2`, static libR, the
  recommended packages included. It records the configure line, `CC`,
  `CFLAGS`, `LDFLAGS` and whether libR is static in
  `PREFIX/robigo-stock.txt`.
- **`perf-rt`:**
  - GNU R's per-signature cache key goes from `rt-gnur-1` to `rt-gnur-2`,
    and it covers `gnur-stock.sh` and `r-src.sh`, so no cached measurement
    of the old rival is used.
  - On a miss the job restores the stock build from its own cache (per
    oracle image and build script; it does not depend on the CPU), or
    builds it. It prints `robigo-stock.txt` to the job summary and
    measures that build with `perf.py rt-gnur` (`RSCRIPT_BIN`,
    `R_HOME_DIR`).
  - A hit needs neither the build nor the cache.
  - **What a miss costs:** the build took 212 s on m0pad's 12 cores and
    333 s on 6 of the desktop's. On a 4-vCPU runner, expect about ten
    minutes, once per oracle image. The install is 103 MB. A hit leaves
    perf-rt at about a minute, as today.
- **`benchmarks/rt/m1-bars.toml`:**
  - `[gnur]` holds the stock build's numbers, measured on the desktop for
    local runs (below);
  - its source is this entry;
  - s3's bar is `met = false`.
- **The gate needs no code change.** An unmet bar fails only on a slowdown
  of more than 0.5% against main's binary, or on an accepted regression's
  ceiling (s3 has none), exactly as fib's bar. The ratio itself is only
  reported, as "not met yet". `scripts/test-perf-rt-bars.py` gains a test on
  the committed bars file: s3 at 1.096x the stock numbers passes, and s3
  0.6% slower than main fails.

### GNU R's numbers

These were measured on the desktop (AMD Ryzen 9 5900X, signature
`1be0f69f7d2b7252`) in the oracle image, with Debian's valgrind 3.24.0
installed (libc6 2.41-12+deb13u4), as `perf-rt` does.
- **Which tool for which rows:**
  - the six rows with bars against GNU R and the three RSS programs:
    `perf.py rt-gnur`, as CI measures them;
  - the five rows it does not measure (fib, binarytrees, the e7 rows):
    the bars tool.
- **Reproduction:** each was run twice, and the runs agree within 0.0006%.
- **Two discrepancies, both too small to matter:**
  - `rt-gnur` and the bars tool called directly differ on s3 by 0.08% on
    one build: 17,408.4 against 17,395.4;
  - two builds of the same configure differ by 0.01% under `rt-gnur`:
    17,408.4 against 17,410.4.

  The committed numbers are `rt-gnur`'s.

| row | oracle image's build (before) | literal default (after) | change |
|---|---:|---:|---:|
| plain | 4,799.6 | 4,275.2 | −10.9% |
| defaults | 6,088.2 | 5,448.9 | −10.5% |
| named | 4,639.3 | 4,165.0 | −10.2% |
| env-fn | 3,832.0 | 3,388.8 | −11.6% |
| superassign | 7,838.1 | 6,892.5 | −12.1% |
| s3 | 19,213.2 | 17,408.4 | −9.4% |
| fib | 24,056,137.6 | 21,255,286.6 | −11.6% |
| binarytrees | 3,781,021.1 | 3,373,953.6 | −10.8% |
| e7-paste | 195,871.2 | 177,076.3 | −9.6% |
| e7-match | 207,794.5 | 203,414.4 | −2.1% |
| e7-unique | 170,862.1 | 157,942.3 | −7.6% |

| program | peak RSS KiB, before | after |
|---|---:|---:|
| `suite/binarytrees.R` | 42,060 | 42,428 |
| `rt/mem/listgrow.R` | 54,160 | 54,220 |
| `rt/mem/churn.R` | 134,392 | 134,684 |

The e7 rows hash strings by address, so their GNU R counts carry up to 5%
of layout noise (A0, "The hashing rows"). Their bars compare with today's
robigo, so the noise moves no bar.

### Every bar, before and after

The same robigo-rt binary (release, main `2ae1e075`) was measured on the
desktop in the oracle image with `perf.py rt-bars --cycles`, once against
the old committed GNU R numbers and once against the stock build's
(`bars-before.md`, `bars-after.md`):

| bar | max | robigo-rt | ratio before | ratio after | before | after |
|---|---:|---:|---:|---:|---|---|
| plain | 0.6x gnur | 1,922.1 | 0.400x | 0.450x | met | met |
| defaults | 0.6x gnur | 1,999.0 | 0.328x | 0.367x | met | met |
| named | 0.6x gnur | 1,848.1 | 0.398x | 0.444x | met | met |
| env-fn | 1.0x gnur | 2,592.6 | 0.677x | 0.765x | met | met |
| superassign | 1.0x gnur | 5,142.6 | 0.656x | 0.746x | met | met |
| **s3** | 1.0x gnur | 19,053.6 | 0.992x | **1.095x** | met | **unmet, owed by P3** |
| fib | 1.0x today | 10,723,872.1 | 1.272x | 1.272x | unmet (G8) | unmet (G8) |
| binarytrees | 1.0x today | 3,485,419.1 | 0.842x | 0.842x | met | met |
| e7-paste | 1.0x today | 96,176.8 | 1.930x | 1.930x | accepted to 2.0x | accepted to 2.0x |
| e7-match | 1.0x today | 99,106.8 | 0.866x | 0.866x | met | met |
| e7-unique | 1.0x today | 127,188.4 | 0.701x | 0.701x | met | met |

| RSS bar | robigo-rt KiB | GNU R before | GNU R after | status |
|---|---:|---:|---:|---|
| `suite/binarytrees.R` (also ≤ today's 32,156) | 24,012 | 42,060 | 42,428 | met |
| `rt/mem/listgrow.R` | 45,516 | 54,160 | 54,220 | met |
| `rt/mem/churn.R` | 73,536 | 134,392 | 134,684 | met |

- **Only s3 flips.** It owes 9.5%: robigo-rt's 19,053.6 against the
  rival's 17,408.4.
- **m0pad agrees:** A0's robigo-rt against the literal default built there
  (`rt-gnur-stock-m0pad.json`) reads 1.095x.
- **Everything else holds:**
  - the other E1 bars stay met, the closest being env-fn at 0.765x;
  - the RSS bars stay met;
  - E2's and E7's bars compare with today's robigo, so they do not move.
- **The gate on CI** measures the rival per CPU signature on the gating
  runner. This PR's own run is its first measurement there.

### What it means for the plan

- **s3's thin margin** (0.9%, the plan's §A.4) is gone with its met bar.
  s3 is now held to the 0.5% rule against main until P3 closes it.
- **e7-paste** (1.930x against its 2.0x ceiling) compares with today's
  robigo and is unchanged.
- **P3's debt** on s3 is about 9.5%, not the plan's "about 1.01x". A0's
  cause tables show where the rival got cheaper: eval.c, envir.c, match.c
  and objects.c each fell by 10% to 13.5%, the files the static build
  shrank most. They profile GNU R only, so they say nothing about where
  robigo-rt's own instructions go.

