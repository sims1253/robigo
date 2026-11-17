### robigo-rt's M1 bars (scripts/perf.py rt-bars)

CPU: AMD Ryzen 9 5900X 12-Core Processor, signature 1be0f69f7d2b7252.
robigo-rt: bins/s3x-a6: sha256 09cd558d3f867750, build ID ae223bd00fa98e39, modified 2026-11-17T08:01:48Z; checkout nightly-2026-11-16-368-g64c3e97f
main: bins/main-a9c4: sha256 8c2f0fbd6119e361, build ID 23991091d8f19f20, modified 2026-11-17T06:05:35Z; checkout nightly-2026-11-16-368-g64c3e97f
Callgrind instructions per iteration; bars against docs/ledger/g3/2026-11-17-a0b-stock-rival.md (measured on another machine) and today's robigo.
Against main: main's binary of the merge's base, on this runner.

| row | instructions | main | change | against | ratio | bar | |
|---|---:|---:|---:|---|---:|---:|---|
| plain | 1,918.3 | 1,918.1 | +0.01% | gnur | 0.449x | 0.6x | ok |
| defaults | 1,998.7 | 1,998.6 | +0.01% | gnur | 0.367x | 0.6x | ok |
| named | 1,844.6 | 1,845.1 | -0.02% | gnur | 0.443x | 0.6x | ok |
| env-fn | 1,942.8 | 1,942.6 | +0.01% | gnur | 0.573x | 1.0x | ok |
| superassign | 4,910.5 | 4,910.6 | -0.00% | gnur | 0.712x | 1.0x | ok |
| s3 | 18,775.1 | 18,776.1 | -0.01% | gnur | 1.079x | (1.0x) | not met yet |
| fib | 10,723,075.8 | 10,723,131.7 | -0.00% | today | 1.272x | (1.0x) | not met yet |
| binarytrees | 3,480,469.2 | 3,480,475.1 | -0.00% | today | 0.841x | 1.0x | ok |
| e7-paste | 96,110.3 | 96,106.5 | +0.00% | today | 1.929x | (1.0x) | accepted up to 2.0x |
| e7-match | 92,235.2 | 92,290.3 | -0.06% | today | 0.806x | 1.0x | ok |
| e7-unique | 123,004.8 | 122,980.9 | +0.02% | today | 0.678x | 1.0x | ok |

Address-sensitive rows: the median over heap shifts k ∈ {0, 300, 1000, 3000, 5000} (k unrelated strings before the setup), with the range over them:
- e7-match: 91,939.8–96,008.0; main 91,931.3–96,059.1

| program | peak RSS KiB | main | bars |
|---|---:|---:|---|
| benchmarks/suite/binarytrees.R | 26,568 | 26,632 | ≤ today 32,156, gnur 42,428 |
| benchmarks/rt/mem/listgrow.R | 46,040 | 45,940 | ≤ gnur 54,220 |
| benchmarks/rt/mem/churn.R | 74,152 | 74,308 | ≤ gnur 134,684 |

(A bar in parentheses is not met yet: only regressions against main fail it, and an accepted regression's ceiling.)

0 failure(s); thresholds: 0.5% instructions, 2.0% peak RSS against main
