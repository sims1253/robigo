### robigo-rt's M1 bars (scripts/perf.py rt-bars)

CPU: AMD Ryzen 9 5900X 12-Core Processor, signature 1be0f69f7d2b7252.
robigo-rt: bins/s3x-a-773m: sha256 d9cafa9a5390c956, build ID d0585aa19e9afb38, modified 2026-11-17T09:12:32Z; checkout nightly-2026-11-16-424-g445747c2
main: bins/main-78dc: sha256 c17914827ccebc43, build ID f5755fcdc1b24695, modified 2026-11-17T09:19:31Z; checkout nightly-2026-11-16-424-g445747c2
Callgrind instructions per iteration; bars against docs/ledger/g3/2026-11-17-a0b-stock-rival.md (measured on another machine) and today's robigo.
Against main: main's binary of the merge's base, on this runner.

| row | instructions | main | change | against | ratio | bar | |
|---|---:|---:|---:|---|---:|---:|---|
| plain | 1,918.2 | 1,918.2 | +0.00% | gnur | 0.449x | 0.6x | ok |
| defaults | 1,998.4 | 1,999.8 | -0.07% | gnur | 0.367x | 0.6x | ok |
| named | 1,844.1 | 1,844.3 | -0.01% | gnur | 0.443x | 0.6x | ok |
| env-fn | 1,943.0 | 1,942.5 | +0.03% | gnur | 0.573x | 1.0x | ok |
| superassign | 4,912.5 | 4,913.2 | -0.02% | gnur | 0.713x | 1.0x | ok |
| s3 | 18,810.9 | 18,810.4 | +0.00% | gnur | 1.081x | (1.0x) | not met yet |
| fib | 10,723,087.9 | 10,723,071.2 | +0.00% | today | 1.272x | (1.0x) | not met yet |
| binarytrees | 3,480,450.4 | 3,480,513.3 | -0.00% | today | 0.841x | 1.0x | ok |
| e7-paste | 96,110.3 | 96,102.7 | +0.01% | today | 1.929x | (1.0x) | accepted up to 2.0x |
| e7-match | 92,264.0 | 92,196.8 | +0.07% | today | 0.806x | 1.0x | ok |
| e7-unique | 123,008.9 | 123,009.5 | -0.00% | today | 0.678x | 1.0x | ok |

Address-sensitive rows: the median over heap shifts k ∈ {0, 300, 1000, 3000, 5000} (k unrelated strings before the setup), with the range over them:
- e7-match: 91,903.4–96,068.9; main 91,861.2–95,954.2

| program | peak RSS KiB | main | bars |
|---|---:|---:|---|
| benchmarks/suite/binarytrees.R | 26,460 | 26,556 | ≤ today 32,156, gnur 42,428 |
| benchmarks/rt/mem/listgrow.R | 46,208 | 46,444 | ≤ gnur 54,220 |
| benchmarks/rt/mem/churn.R | 74,324 | 74,216 | ≤ gnur 134,684 |

(A bar in parentheses is not met yet: only regressions against main fail it, and an accepted regression's ceiling.)

0 failure(s); thresholds: 0.5% instructions, 2.0% peak RSS against main
