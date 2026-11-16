### robigo-rt's M1 bars (scripts/perf.py rt-bars)

CPU: AMD Ryzen 9 5900X 12-Core Processor, signature 1be0f69f7d2b7252.
Callgrind instructions per iteration; bars against docs/ledger/g3/2026-11-16-bars-oracle-baseline.md (measured on another machine) and today's robigo.
Against main: none (the bars only).
Estimated cycles under `--I1=32768,8,64 --D1=32768,8,64 --LL=33554432,16,64`: reported only, never failed on.

| row | instructions | main | change | against | ratio | bar | | est. cycles | cycles change |
|---|---:|---:|---:|---|---:|---:|---|---:|---:|
| plain | 1,922.2 |  |  | gnur | 0.400x | 0.6x | ok | 1,937.1 |  |
| defaults | 1,999.0 |  |  | gnur | 0.328x | 0.6x | ok | 2,013.9 |  |
| named | 1,848.1 |  |  | gnur | 0.398x | 0.6x | ok | 1,863.0 |  |
| env-fn | 2,592.6 |  |  | gnur | 0.677x | 1.0x | ok | 2,607.5 |  |
| superassign | 5,142.6 |  |  | gnur | 0.656x | 1.0x | ok | 5,173.9 |  |
| s3 | 19,053.3 |  |  | gnur | 0.992x | (1.0x) | ok | 21,828.0 |  |
| fib | 10,723,889.6 |  |  | today | 1.272x | (1.0x) | not met yet | 10,826,187.1 |  |
| binarytrees | 3,485,419.5 |  |  | today | 0.842x | 1.0x | ok | 3,629,148.7 |  |
| e7-paste | 96,174.8 |  |  | today | 1.930x | (1.0x) | accepted up to 2.0x | 124,824.4 |  |
| e7-match | 99,103.7 |  |  | today | 0.866x | 1.0x | ok | 111,636.5 |  |
| e7-unique | 127,188.8 |  |  | today | 0.701x | 1.0x | ok | 135,521.0 |  |

| program | peak RSS KiB | main | bars |
|---|---:|---:|---|
| benchmarks/suite/binarytrees.R | 23,880 |  | ≤ today 32,156, gnur 42,060 |
| benchmarks/rt/mem/listgrow.R | 45,564 |  | ≤ gnur 54,160 |
| benchmarks/rt/mem/churn.R | 73,672 |  | ≤ gnur 134,392 |

(A bar in parentheses is not met yet: only regressions against main fail it, and an accepted regression's ceiling.)

NOTE `s3`: 0.992x gnur meets its bar of 1.0x, so set it to met in benchmarks/rt/m1-bars.toml

0 failure(s); thresholds: 0.5% instructions, 2.0% peak RSS against main
