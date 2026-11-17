### robigo-rt's M1 bars (scripts/perf.py rt-bars)

CPU: AMD Ryzen 9 5900X 12-Core Processor, signature 1be0f69f7d2b7252.
robigo-rt: bw-n: sha256 c14f910f87297d92, build ID 30a4aaa6e180c8e2, modified 2026-11-17T03:22:52Z; checkout nightly-2026-11-16-290-g3c734519
main: main-c4a6: sha256 c70f0a3b2ef28b16, build ID 984a7eb7f6043ac8, modified 2026-11-17T03:22:28Z; checkout nightly-2026-11-16-290-g3c734519
Callgrind instructions per iteration; bars against docs/ledger/g3/2026-11-17-a0b-stock-rival.md (measured on another machine) and today's robigo.
Against main: main's binary of the merge's base, on this runner.

| row | instructions | main | change | against | ratio | bar | |
|---|---:|---:|---:|---|---:|---:|---|
| plain | 1,917.7 | 1,918.1 | -0.02% | gnur | 0.449x | 0.6x | ok |
| defaults | 1,998.8 | 1,999.0 | -0.01% | gnur | 0.367x | 0.6x | ok |
| named | 1,843.8 | 1,844.1 | -0.02% | gnur | 0.443x | 0.6x | ok |
| env-fn | 1,942.9 | 2,503.6 | -22.39% | gnur | 0.573x | 1.0x | ok |
| superassign | 4,939.9 | 4,937.9 | +0.04% | gnur | 0.717x | 1.0x | ok |
| s3 | 18,806.4 | 18,747.5 | +0.31% | gnur | 1.080x | (1.0x) | not met yet |
| fib | 10,723,452.7 | 10,723,361.8 | +0.00% | today | 1.272x | (1.0x) | not met yet |
| binarytrees | 3,481,596.3 | 3,481,600.9 | -0.00% | today | 0.841x | 1.0x | ok |
| e7-paste | 96,113.3 | 96,049.7 | +0.07% | today | 1.929x | (1.0x) | accepted up to 2.0x |
| e7-match | 92,279.6 | 92,020.3 | +0.28% | today | 0.807x | 1.0x | ok |
| e7-unique | 122,993.3 | 123,003.4 | -0.01% | today | 0.678x | 1.0x | ok |

Address-sensitive rows: the median over heap shifts k ∈ {0, 300, 1000, 3000, 5000} (k unrelated strings before the setup), with the range over them:
- e7-match: 91,911.7–96,111.9; main 91,887.4–92,250.8

| program | peak RSS KiB | main | bars |
|---|---:|---:|---|
| benchmarks/suite/binarytrees.R | 26,552 | 26,500 | ≤ today 32,156, gnur 42,428 |
| benchmarks/rt/mem/listgrow.R | 45,288 | 45,392 | ≤ gnur 54,220 |
| benchmarks/rt/mem/churn.R | 73,300 | 73,856 | ≤ gnur 134,684 |

(A bar in parentheses is not met yet: only regressions against main fail it, and an accepted regression's ceiling.)

0 failure(s); thresholds: 0.5% instructions, 2.0% peak RSS against main
