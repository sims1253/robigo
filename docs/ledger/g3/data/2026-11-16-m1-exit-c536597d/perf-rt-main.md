# perf-rt on main c536597d: run 43918105805, job 133477890224 (ubuntu-24.04 runner, CI's Rust image, Debian trixie)
### robigo-rt's M1 bars (scripts/perf.py rt-bars)

Callgrind instructions per iteration; bars against docs/ledger/g3/2026-11-16-bars-oracle-baseline.md and today's robigo.
Against main: none (the bars only).

| row | instructions | main | change | against | ratio | bar | |
|---|---:|---:|---:|---|---:|---:|---|
| plain | 1,923.0 |  |  | gnur | 0.401x | 0.6x | ok |
| defaults | 2,000.0 |  |  | gnur | 0.328x | 0.6x | ok |
| named | 1,849.1 |  |  | gnur | 0.399x | 0.6x | ok |
| env-fn | 2,592.6 |  |  | gnur | 0.677x | 1.0x | ok |
| superassign | 5,144.4 |  |  | gnur | 0.656x | 1.0x | ok |
| s3 | 19,043.5 |  |  | gnur | 0.991x | 1.0x | ok |
| fib | 10,722,529.1 |  |  | today | 1.272x | (1.0x) | not met yet |
| binarytrees | 3,476,183.4 |  |  | today | 0.840x | 1.0x | ok |
| e7-paste | 96,145.1 |  |  | today | 1.930x | (1.0x) | accepted up to 2.0x |
| e7-match | 99,063.7 |  |  | today | 0.866x | 1.0x | ok |
| e7-unique | 126,765.7 |  |  | today | 0.699x | 1.0x | ok |

| program | peak RSS KiB | main | bars |
|---|---:|---:|---|
| benchmarks/suite/binarytrees.R | 23,704 |  | ≤ today 32,156, gnur 42,060 |
| benchmarks/rt/mem/listgrow.R | 45,352 |  | ≤ gnur 54,160 |
| benchmarks/rt/mem/churn.R | 73,960 |  | ≤ gnur 134,392 |

(A bar in parentheses is not met yet: only regressions against main fail it, and an accepted regression's ceiling.)

0 failure(s); thresholds: 0.5% instructions, 2.0% peak RSS against main
