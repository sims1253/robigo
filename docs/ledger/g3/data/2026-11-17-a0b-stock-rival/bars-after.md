### robigo-rt's M1 bars (scripts/perf.py rt-bars)

CPU: AMD Ryzen 9 5900X 12-Core Processor, signature 1be0f69f7d2b7252.
Callgrind instructions per iteration; bars against GNU R 4.6.2 measured on this CPU signature (/o/rt-gnur-final-run1.json) and today's robigo.
Against main: none (the bars only).
Estimated cycles under `--I1=32768,8,64 --D1=32768,8,64 --LL=33554432,16,64`: reported only, never failed on.

| row | instructions | main | change | against | ratio | bar | | est. cycles | cycles change |
|---|---:|---:|---:|---|---:|---:|---|---:|---:|
| plain | 1,922.1 |  |  | gnur | 0.450x | 0.6x | ok | 1,937.0 |  |
| defaults | 1,999.0 |  |  | gnur | 0.367x | 0.6x | ok | 2,014.0 |  |
| named | 1,848.1 |  |  | gnur | 0.444x | 0.6x | ok | 1,862.9 |  |
| env-fn | 2,592.6 |  |  | gnur | 0.765x | 1.0x | ok | 2,607.5 |  |
| superassign | 5,142.6 |  |  | gnur | 0.746x | 1.0x | ok | 5,173.9 |  |
| s3 | 19,053.6 |  |  | gnur | 1.095x | (1.0x) | not met yet | 21,828.1 |  |
| fib | 10,723,872.1 |  |  | today | 1.272x | (1.0x) | not met yet | 10,826,169.1 |  |
| binarytrees | 3,485,419.1 |  |  | today | 0.842x | 1.0x | ok | 3,629,148.4 |  |
| e7-paste | 96,176.8 |  |  | today | 1.930x | (1.0x) | accepted up to 2.0x | 124,828.7 |  |
| e7-match | 99,106.8 |  |  | today | 0.866x | 1.0x | ok | 111,646.3 |  |
| e7-unique | 127,188.4 |  |  | today | 0.701x | 1.0x | ok | 135,520.3 |  |

| program | peak RSS KiB | main | bars |
|---|---:|---:|---|
| benchmarks/suite/binarytrees.R | 24,012 |  | ≤ today 32,156, gnur 42,428 |
| benchmarks/rt/mem/listgrow.R | 45,516 |  | ≤ gnur 54,220 |
| benchmarks/rt/mem/churn.R | 73,536 |  | ≤ gnur 134,684 |

(A bar in parentheses is not met yet: only regressions against main fail it, and an accepted regression's ceiling.)

0 failure(s); thresholds: 0.5% instructions, 2.0% peak RSS against main
