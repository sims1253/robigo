### robigo-rt's M1 bars (scripts/perf.py rt-bars)

CPU: AMD Ryzen 9 5900X 12-Core Processor, signature 1be0f69f7d2b7252.
Callgrind instructions per iteration; bars against docs/ledger/g3/2026-11-16-bars-oracle-baseline.md (measured on another machine) and today's robigo.
Against main: main's binary of the merge's base, on this runner.

| row | instructions | main | change | against | ratio | bar | |
|---|---:|---:|---:|---|---:|---:|---|
| plain | 1,917.8 | 1,918.1 | -0.02% | gnur | 0.400x | 0.6x | ok |
| defaults | 1,999.0 | 1,999.0 | +0.00% | gnur | 0.328x | 0.6x | ok |
| named | 1,844.2 | 1,844.1 | +0.00% | gnur | 0.398x | 0.6x | ok |
| env-fn | 1,942.1 | 2,503.6 | -22.43% | gnur | 0.507x | 1.0x | ok |
| superassign | 4,940.1 | 4,937.9 | +0.05% | gnur | 0.630x | 1.0x | ok |
| s3 | 18,806.2 | 18,747.2 | +0.31% | gnur | 0.979x | 1.0x | ok |
| fib | 10,723,404.5 | 10,723,377.0 | +0.00% | today | 1.272x | (1.0x) | not met yet |
| binarytrees | 3,481,601.9 | 3,481,603.5 | -0.00% | today | 0.841x | 1.0x | ok |
| e7-paste | 96,096.2 | 96,054.2 | +0.04% | today | 1.929x | (1.0x) | accepted up to 2.0x |
| e7-match | 103,148.9 | 99,035.9 | +4.15% | today | 0.902x | 1.0x | ok, SLOWER |
| e7-unique | 126,493.0 | 126,474.7 | +0.01% | today | 0.697x | 1.0x | ok |

| program | peak RSS KiB | main | bars |
|---|---:|---:|---|
| benchmarks/suite/binarytrees.R | 26,620 | 26,528 | ≤ today 32,156, gnur 42,060 |
| benchmarks/rt/mem/listgrow.R | 45,176 | 45,240 | ≤ gnur 54,160 |
| benchmarks/rt/mem/churn.R | 73,384 | 73,616 | ≤ gnur 134,392 |

(A bar in parentheses is not met yet: only regressions against main fail it, and an accepted regression's ceiling.)

FAIL `e7-match`: +4.15% instructions against main (99,035.9 -> 103,148.9)

1 failure(s); thresholds: 0.5% instructions, 2.0% peak RSS against main
