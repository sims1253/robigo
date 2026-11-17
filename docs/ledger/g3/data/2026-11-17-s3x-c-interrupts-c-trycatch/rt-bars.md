### robigo-rt's M1 bars (scripts/perf.py rt-bars)

CPU: AMD Ryzen 9 5900X 12-Core Processor, signature 1be0f69f7d2b7252.
robigo-rt: bins/s3xc-1: sha256 9f4e1c41916a7690, build ID 196bdfdb17004641, modified 2026-11-17T13:44:55Z; checkout nightly-2026-11-17-83-ge9a5da70c
main: bins/main-dce8: sha256 7d14ed44a565aed7, build ID 069974ac3882fbf2, modified 2026-11-17T13:09:04Z; checkout nightly-2026-11-17-83-ge9a5da70c
Callgrind instructions per iteration; bars against docs/ledger/g3/2026-11-17-a0b-stock-rival.md (measured on another machine) and today's robigo.
Against main: main's binary of the merge's base, on this runner.

| row | instructions | main | change | against | ratio | bar | |
|---|---:|---:|---:|---|---:|---:|---|
| plain | 1,918.5 | 1,918.1 | +0.02% | gnur | 0.449x | 0.6x | ok |
| defaults | 1,998.8 | 1,999.3 | -0.03% | gnur | 0.367x | 0.6x | ok |
| named | 1,844.7 | 1,844.5 | +0.01% | gnur | 0.443x | 0.6x | ok |
| env-fn | 1,942.6 | 1,942.8 | -0.01% | gnur | 0.573x | 1.0x | ok |
| superassign | 4,902.4 | 4,910.4 | -0.16% | gnur | 0.711x | 1.0x | ok |
| s3 | 18,804.8 | 18,812.7 | -0.04% | gnur | 1.080x | (1.0x) | not met yet |
| fib | 10,723,031.2 | 10,723,115.0 | -0.00% | today | 1.272x | (1.0x) | not met yet |
| binarytrees | 3,480,634.2 | 3,480,440.6 | +0.01% | today | 0.841x | 1.0x | ok |
| e7-paste | 96,118.2 | 96,136.9 | -0.02% | today | 1.929x | (1.0x) | accepted up to 2.0x |
| e7-match | 92,222.4 | 92,227.8 | -0.01% | today | 0.806x | 1.0x | ok |
| e7-unique | 123,014.8 | 123,008.9 | +0.00% | today | 0.678x | 1.0x | ok |

Address-sensitive rows: the median over heap shifts k ∈ {0, 300, 1000, 3000, 5000} (k unrelated strings before the setup), with the range over them:
- e7-match: 91,908.5–95,983.2; main 91,878.9–95,977.0

| program | peak RSS KiB | main | bars |
|---|---:|---:|---|
| benchmarks/suite/binarytrees.R | 26,628 | 26,636 | ≤ today 32,156, gnur 42,428 |
| benchmarks/rt/mem/listgrow.R | 46,116 | 46,164 | ≤ gnur 54,220 |
| benchmarks/rt/mem/churn.R | 74,472 | 74,420 | ≤ gnur 134,684 |

(A bar in parentheses is not met yet: only regressions against main fail it, and an accepted regression's ceiling.)

0 failure(s); thresholds: 0.5% instructions, 2.0% peak RSS against main
