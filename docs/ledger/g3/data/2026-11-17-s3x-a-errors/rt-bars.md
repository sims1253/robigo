### robigo-rt's M1 bars (scripts/perf.py rt-bars)

CPU: AMD Ryzen 9 5900X 12-Core Processor, signature 1be0f69f7d2b7252.
robigo-rt: s3x-a5: sha256 13d4d2bb266bf236, build ID 5ab4b77927ee51ab, modified 2026-11-17T07:27:29Z; checkout nightly-2026-11-16-364-g4d2e479e
main: main-a9c4: sha256 8c2f0fbd6119e361, build ID 23991091d8f19f20, modified 2026-11-17T06:05:35Z; checkout nightly-2026-11-16-364-g4d2e479e
Callgrind instructions per iteration; bars against docs/ledger/g3/2026-11-17-a0b-stock-rival.md (measured on another machine) and today's robigo.
Against main: main's binary of the merge's base, on this runner.

| row | instructions | main | change | against | ratio | bar | |
|---|---:|---:|---:|---|---:|---:|---|
| plain | 1,917.2 | 1,917.9 | -0.04% | gnur | 0.448x | 0.6x | ok |
| defaults | 1,999.3 | 1,998.6 | +0.03% | gnur | 0.367x | 0.6x | ok |
| named | 1,843.8 | 1,844.1 | -0.02% | gnur | 0.443x | 0.6x | ok |
| env-fn | 1,941.9 | 1,942.1 | -0.01% | gnur | 0.573x | 1.0x | ok |
| superassign | 4,910.7 | 4,910.3 | +0.01% | gnur | 0.712x | 1.0x | ok |
| s3 | 18,775.3 | 18,775.4 | -0.00% | gnur | 1.079x | (1.0x) | not met yet |
| fib | 10,723,041.1 | 10,723,066.1 | -0.00% | today | 1.272x | (1.0x) | not met yet |
| binarytrees | 3,480,494.4 | 3,480,438.1 | +0.00% | today | 0.841x | 1.0x | ok |
| e7-paste | 96,104.8 | 96,110.0 | -0.01% | today | 1.929x | (1.0x) | accepted up to 2.0x |
| e7-match | 92,226.6 | 92,281.1 | -0.06% | today | 0.806x | 1.0x | ok |
| e7-unique | 122,984.7 | 122,994.3 | -0.01% | today | 0.678x | 1.0x | ok |

Address-sensitive rows: the median over heap shifts k ∈ {0, 300, 1000, 3000, 5000} (k unrelated strings before the setup), with the range over them:
- e7-match: 91,930.2–96,018.5; main 91,942.6–96,052.8

| program | peak RSS KiB | main | bars |
|---|---:|---:|---|
| benchmarks/suite/binarytrees.R | 26,476 | 26,548 | ≤ today 32,156, gnur 42,428 |
| benchmarks/rt/mem/listgrow.R | 45,852 | 45,952 | ≤ gnur 54,220 |
| benchmarks/rt/mem/churn.R | 74,104 | 74,216 | ≤ gnur 134,684 |

(A bar in parentheses is not met yet: only regressions against main fail it, and an accepted regression's ceiling.)

0 failure(s); thresholds: 0.5% instructions, 2.0% peak RSS against main
