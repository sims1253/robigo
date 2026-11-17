### robigo-rt's M1 bars (scripts/perf.py rt-bars)

CPU: AMD Ryzen 9 5900X 12-Core Processor, signature 1be0f69f7d2b7252.
Callgrind instructions per iteration; bars against docs/ledger/g3/2026-11-16-bars-oracle-baseline.md (measured on another machine) and today's robigo.
Against main: none (the bars only).
Estimated cycles under `--I1=32768,8,64 --D1=32768,8,64 --LL=33554432,16,64`: reported only, never failed on.

| row | instructions | main | change | against | ratio | bar | | est. cycles | cycles change |
|---|---:|---:|---:|---|---:|---:|---|---:|---:|
| plain | 1,918.1 |  |  | gnur | 0.400x | 0.6x | ok | 1,933.0 |  |
| defaults | 1,999.0 |  |  | gnur | 0.328x | 0.6x | ok | 2,014.2 |  |
| named | 1,844.1 |  |  | gnur | 0.398x | 0.6x | ok | 1,859.0 |  |
| env-fn | 2,588.6 |  |  | gnur | 0.676x | 1.0x | ok | 2,604.5 |  |
| superassign | 5,142.5 |  |  | gnur | 0.656x | 1.0x | ok | 5,174.0 |  |
| s3 | 19,048.5 |  |  | gnur | 0.991x | 1.0x | ok | 21,889.5 |  |
| fib | 10,723,203.6 |  |  | today | 1.272x | (1.0x) | not met yet | 10,826,694.6 |  |
| binarytrees | 3,481,843.5 |  |  | today | 0.841x | 1.0x | ok | 3,637,684.0 |  |
| e7-paste | 96,170.5 |  |  | today | 1.930x | (1.0x) | accepted up to 2.0x | 124,820.5 |  |
| e7-match | 99,109.3 |  |  | today | 0.866x | 1.0x | ok | 111,664.7 |  |
| e7-unique | 127,187.1 |  |  | today | 0.701x | 1.0x | ok | 135,512.1 |  |

| program | peak RSS KiB | main | bars |
|---|---:|---:|---|
| benchmarks/suite/binarytrees.R | 23,816 |  | ≤ today 32,156, gnur 42,060 |
| benchmarks/rt/mem/listgrow.R | 45,768 |  | ≤ gnur 54,160 |
| benchmarks/rt/mem/churn.R | 73,732 |  | ≤ gnur 134,392 |

(A bar in parentheses is not met yet: only regressions against main fail it, and an accepted regression's ceiling.)

0 failure(s); thresholds: 0.5% instructions, 2.0% peak RSS against main
