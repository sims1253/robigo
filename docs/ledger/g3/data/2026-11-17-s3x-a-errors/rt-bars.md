### robigo-rt's M1 bars (scripts/perf.py rt-bars)

CPU: AMD Ryzen 9 5900X 12-Core Processor, signature 1be0f69f7d2b7252.
robigo-rt: bins/s3xa-m2: sha256 78c0a56ba03aa599, build ID 435a3fef908289a2, modified 2026-11-17T10:32:24Z; checkout nightly-2026-11-16-423-gc07d8c2a
main: bins/main-4804: sha256 1c0278d67c8eca13, build ID 5cfed601c84831fc, modified 2026-11-17T10:39:56Z; checkout nightly-2026-11-16-423-gc07d8c2a
Callgrind instructions per iteration; bars against docs/ledger/g3/2026-11-17-a0b-stock-rival.md (measured on another machine) and today's robigo.
Against main: main's binary of the merge's base, on this runner.

| row | instructions | main | change | against | ratio | bar | |
|---|---:|---:|---:|---|---:|---:|---|
| plain | 1,918.3 | 1,918.2 | +0.01% | gnur | 0.449x | 0.6x | ok |
| defaults | 1,999.0 | 1,998.9 | +0.00% | gnur | 0.367x | 0.6x | ok |
| named | 1,843.7 | 1,843.9 | -0.01% | gnur | 0.443x | 0.6x | ok |
| env-fn | 1,942.6 | 1,942.2 | +0.02% | gnur | 0.573x | 1.0x | ok |
| superassign | 4,912.6 | 4,912.2 | +0.01% | gnur | 0.713x | 1.0x | ok |
| s3 | 18,810.3 | 18,810.1 | +0.00% | gnur | 1.081x | (1.0x) | not met yet |
| fib | 10,723,097.8 | 10,723,015.3 | +0.00% | today | 1.272x | (1.0x) | not met yet |
| binarytrees | 3,480,484.5 | 3,480,459.6 | +0.00% | today | 0.841x | 1.0x | ok |
| e7-paste | 96,133.3 | 96,107.4 | +0.03% | today | 1.930x | (1.0x) | accepted up to 2.0x |
| e7-match | 92,282.6 | 92,199.5 | +0.09% | today | 0.807x | 1.0x | ok |
| e7-unique | 123,018.5 | 123,019.6 | -0.00% | today | 0.678x | 1.0x | ok |

Address-sensitive rows: the median over heap shifts k ∈ {0, 300, 1000, 3000, 5000} (k unrelated strings before the setup), with the range over them:
- e7-match: 91,946.3–96,089.1; main 91,864.1–94,640.5

| program | peak RSS KiB | main | bars |
|---|---:|---:|---|
| benchmarks/suite/binarytrees.R | 26,568 | 26,584 | ≤ today 32,156, gnur 42,428 |
| benchmarks/rt/mem/listgrow.R | 46,272 | 46,204 | ≤ gnur 54,220 |
| benchmarks/rt/mem/churn.R | 74,392 | 74,304 | ≤ gnur 134,684 |

(A bar in parentheses is not met yet: only regressions against main fail it, and an accepted regression's ceiling.)

0 failure(s); thresholds: 0.5% instructions, 2.0% peak RSS against main
