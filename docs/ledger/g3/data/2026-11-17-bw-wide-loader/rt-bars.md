### robigo-rt's M1 bars (scripts/perf.py rt-bars)

CPU: AMD Ryzen 9 5900X 12-Core Processor, signature 1be0f69f7d2b7252.
robigo-rt: bw-t: sha256 7cd2769ab7ed97f0, build ID 8093f9df78519c06, modified 2026-11-17T05:37:35Z; checkout nightly-2026-11-16-351-g0023e17f
main: main-908c: sha256 f24cdde4c2c3bec4, build ID b71c5ff26557965b, modified 2026-11-17T05:38:11Z; checkout nightly-2026-11-16-351-g0023e17f
Callgrind instructions per iteration; bars against docs/ledger/g3/2026-11-17-a0b-stock-rival.md (measured on another machine) and today's robigo.
Against main: main's binary of the merge's base, on this runner.

| row | instructions | main | change | against | ratio | bar | |
|---|---:|---:|---:|---|---:|---:|---|
| plain | 1,918.2 | 1,918.1 | +0.00% | gnur | 0.449x | 0.6x | ok |
| defaults | 1,998.8 | 1,999.0 | -0.01% | gnur | 0.367x | 0.6x | ok |
| named | 1,844.1 | 1,844.2 | -0.01% | gnur | 0.443x | 0.6x | ok |
| env-fn | 1,942.6 | 2,484.5 | -21.81% | gnur | 0.573x | 1.0x | ok |
| superassign | 4,910.9 | 4,896.9 | +0.28% | gnur | 0.712x | 1.0x | ok |
| s3 | 18,776.3 | 18,722.5 | +0.29% | gnur | 1.079x | (1.0x) | not met yet |
| fib | 10,723,035.0 | 10,723,102.3 | -0.00% | today | 1.272x | (1.0x) | not met yet |
| binarytrees | 3,480,449.0 | 3,480,440.6 | +0.00% | today | 0.841x | 1.0x | ok |
| e7-paste | 96,117.4 | 96,059.3 | +0.06% | today | 1.929x | (1.0x) | accepted up to 2.0x |
| e7-match | 92,284.1 | 92,184.0 | +0.11% | today | 0.807x | 1.0x | ok |
| e7-unique | 123,001.0 | 123,008.8 | -0.01% | today | 0.678x | 1.0x | ok |

Address-sensitive rows: the median over heap shifts k ∈ {0, 300, 1000, 3000, 5000} (k unrelated strings before the setup), with the range over them:
- e7-match: 91,930.4–96,071.2; main 91,914.7–92,314.0

| program | peak RSS KiB | main | bars |
|---|---:|---:|---|
| benchmarks/suite/binarytrees.R | 26,516 | 26,596 | ≤ today 32,156, gnur 42,428 |
| benchmarks/rt/mem/listgrow.R | 46,240 | 46,324 | ≤ gnur 54,220 |
| benchmarks/rt/mem/churn.R | 74,180 | 75,128 | ≤ gnur 134,684 |

(A bar in parentheses is not met yet: only regressions against main fail it, and an accepted regression's ceiling.)

0 failure(s); thresholds: 0.5% instructions, 2.0% peak RSS against main
