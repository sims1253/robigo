### robigo-rt's M1 bars (scripts/perf.py rt-bars)

CPU: AMD Ryzen 9 5900X 12-Core Processor, signature 1be0f69f7d2b7252.
robigo-rt: bins/s3xb-2: sha256 19a213bde3ba311b, build ID c76a90541c0bb491, modified 2026-11-17T13:01:32Z; checkout nightly-2026-11-17-79-g38b186792
main: bins/main-dce8: sha256 7d14ed44a565aed7, build ID 069974ac3882fbf2, modified 2026-11-17T13:09:04Z; checkout nightly-2026-11-17-79-g38b186792
Callgrind instructions per iteration; bars against docs/ledger/g3/2026-11-17-a0b-stock-rival.md (measured on another machine) and today's robigo.
Against main: main's binary of the merge's base, on this runner.

| row | instructions | main | change | against | ratio | bar | |
|---|---:|---:|---:|---|---:|---:|---|
| plain | 1,918.0 | 1,917.9 | +0.01% | gnur | 0.449x | 0.6x | ok |
| defaults | 1,998.8 | 1,998.9 | -0.01% | gnur | 0.367x | 0.6x | ok |
| named | 1,844.0 | 1,844.2 | -0.01% | gnur | 0.443x | 0.6x | ok |
| env-fn | 1,944.9 | 1,942.6 | +0.12% | gnur | 0.574x | 1.0x | ok |
| superassign | 4,902.8 | 4,910.5 | -0.16% | gnur | 0.711x | 1.0x | ok |
| s3 | 18,804.2 | 18,812.8 | -0.05% | gnur | 1.080x | (1.0x) | not met yet |
| fib | 10,723,034.3 | 10,723,176.1 | -0.00% | today | 1.272x | (1.0x) | not met yet |
| binarytrees | 3,480,576.0 | 3,480,503.6 | +0.00% | today | 0.841x | 1.0x | ok |
| e7-paste | 96,119.7 | 96,134.1 | -0.01% | today | 1.929x | (1.0x) | accepted up to 2.0x |
| e7-match | 92,236.9 | 92,231.7 | +0.01% | today | 0.806x | 1.0x | ok |
| e7-unique | 123,013.4 | 123,016.9 | -0.00% | today | 0.678x | 1.0x | ok |

Address-sensitive rows: the median over heap shifts k ∈ {0, 300, 1000, 3000, 5000} (k unrelated strings before the setup), with the range over them:
- e7-match: 91,871.5–94,668.5; main 91,884.3–95,963.1

| program | peak RSS KiB | main | bars |
|---|---:|---:|---|
| benchmarks/suite/binarytrees.R | 26,556 | 26,592 | ≤ today 32,156, gnur 42,428 |
| benchmarks/rt/mem/listgrow.R | 46,136 | 46,380 | ≤ gnur 54,220 |
| benchmarks/rt/mem/churn.R | 74,396 | 74,364 | ≤ gnur 134,684 |

(A bar in parentheses is not met yet: only regressions against main fail it, and an accepted regression's ceiling.)

0 failure(s); thresholds: 0.5% instructions, 2.0% peak RSS against main
