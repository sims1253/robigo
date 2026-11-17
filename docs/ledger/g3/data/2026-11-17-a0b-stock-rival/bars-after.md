### robigo-rt's M1 bars (scripts/perf.py rt-bars)

CPU: AMD Ryzen 9 5900X 12-Core Processor, signature 1be0f69f7d2b7252.
Callgrind instructions per iteration; bars against docs/ledger/g3/2026-11-17-a0b-stock-rival.md (measured on another machine) and today's robigo.
Against main: none (the bars only).
Estimated cycles under `--I1=32768,8,64 --D1=32768,8,64 --LL=33554432,16,64`: reported only, never failed on.

| row | instructions | main | change | against | ratio | bar | | est. cycles | cycles change |
|---|---:|---:|---:|---|---:|---:|---|---:|---:|
| plain | 1,918.1 |  |  | gnur | 0.449x | 0.6x | ok | 1,933.1 |  |
| defaults | 1,999.0 |  |  | gnur | 0.367x | 0.6x | ok | 2,014.1 |  |
| named | 1,844.1 |  |  | gnur | 0.443x | 0.6x | ok | 1,859.0 |  |
| env-fn | 2,588.6 |  |  | gnur | 0.764x | 1.0x | ok | 2,604.5 |  |
| superassign | 5,142.6 |  |  | gnur | 0.746x | 1.0x | ok | 5,174.1 |  |
| s3 | 19,048.5 |  |  | gnur | 1.094x | (1.0x) | not met yet | 21,889.6 |  |
| fib | 10,723,194.2 |  |  | today | 1.272x | (1.0x) | not met yet | 10,826,688.2 |  |
| binarytrees | 3,481,838.8 |  |  | today | 0.841x | 1.0x | ok | 3,637,684.6 |  |
| e7-paste | 96,174.9 |  |  | today | 1.930x | (1.0x) | accepted up to 2.0x | 124,825.3 |  |
| e7-match | 99,109.5 |  |  | today | 0.866x | 1.0x | ok | 111,668.0 |  |
| e7-unique | 127,185.5 |  |  | today | 0.701x | 1.0x | ok | 135,508.8 |  |

| program | peak RSS KiB | main | bars |
|---|---:|---:|---|
| benchmarks/suite/binarytrees.R | 23,712 |  | ≤ today 32,156, gnur 42,428 |
| benchmarks/rt/mem/listgrow.R | 45,616 |  | ≤ gnur 54,220 |
| benchmarks/rt/mem/churn.R | 73,900 |  | ≤ gnur 134,684 |

(A bar in parentheses is not met yet: only regressions against main fail it, and an accepted regression's ceiling.)

0 failure(s); thresholds: 0.5% instructions, 2.0% peak RSS against main
