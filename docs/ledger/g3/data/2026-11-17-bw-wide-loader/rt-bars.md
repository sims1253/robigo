### robigo-rt's M1 bars (scripts/perf.py rt-bars)

CPU: AMD Ryzen 9 5900X 12-Core Processor, signature 1be0f69f7d2b7252.
robigo-rt: bw-o: sha256 7c2f1c35158d2905, build ID 6f9fcc58a20f493c, modified 2026-11-17T03:49:24Z; checkout nightly-2026-11-16-303-gd6148372
main: main-6bfb: sha256 0307fcd794173b57, build ID cfae515eab1a53b9, modified 2026-11-17T03:49:57Z; checkout nightly-2026-11-16-303-gd6148372
Callgrind instructions per iteration; bars against docs/ledger/g3/2026-11-17-a0b-stock-rival.md (measured on another machine) and today's robigo.
Against main: main's binary of the merge's base, on this runner.

| row | instructions | main | change | against | ratio | bar | |
|---|---:|---:|---:|---|---:|---:|---|
| plain | 1,917.9 | 1,918.1 | -0.01% | gnur | 0.449x | 0.6x | ok |
| defaults | 1,999.2 | 1,999.0 | +0.01% | gnur | 0.367x | 0.6x | ok |
| named | 1,843.9 | 1,844.2 | -0.01% | gnur | 0.443x | 0.6x | ok |
| env-fn | 1,942.3 | 2,503.6 | -22.42% | gnur | 0.573x | 1.0x | ok |
| superassign | 4,940.8 | 4,928.8 | +0.24% | gnur | 0.717x | 1.0x | ok |
| s3 | 18,815.8 | 18,764.2 | +0.28% | gnur | 1.081x | (1.0x) | not met yet |
| fib | 10,723,365.9 | 10,723,397.8 | -0.00% | today | 1.272x | (1.0x) | not met yet |
| binarytrees | 3,481,610.9 | 3,481,611.0 | -0.00% | today | 0.841x | 1.0x | ok |
| e7-paste | 96,123.6 | 96,064.7 | +0.06% | today | 1.929x | (1.0x) | accepted up to 2.0x |
| e7-match | 92,304.6 | 92,197.7 | +0.12% | today | 0.807x | 1.0x | ok |
| e7-unique | 123,005.2 | 123,007.4 | -0.00% | today | 0.678x | 1.0x | ok |

Address-sensitive rows: the median over heap shifts k ∈ {0, 300, 1000, 3000, 5000} (k unrelated strings before the setup), with the range over them:
- e7-match: 91,891.2–96,136.1; main 91,898.3–92,290.9

| program | peak RSS KiB | main | bars |
|---|---:|---:|---|
| benchmarks/suite/binarytrees.R | 26,484 | 26,440 | ≤ today 32,156, gnur 42,428 |
| benchmarks/rt/mem/listgrow.R | 45,672 | 45,748 | ≤ gnur 54,220 |
| benchmarks/rt/mem/churn.R | 73,860 | 74,372 | ≤ gnur 134,684 |

(A bar in parentheses is not met yet: only regressions against main fail it, and an accepted regression's ceiling.)

0 failure(s); thresholds: 0.5% instructions, 2.0% peak RSS against main
