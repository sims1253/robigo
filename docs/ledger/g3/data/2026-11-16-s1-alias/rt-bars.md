# perf.py rt-bars at the head after merging main 17ed87e9, against main 17ed87e9 (the merge's base)

### robigo-rt's M1 bars (scripts/perf.py rt-bars)

Callgrind instructions per iteration; bars against docs/ledger/g3/2026-11-16-bars-oracle-baseline.md and today's robigo.
Against main: main's binary of the merge's base, on this runner.

| row | instructions | main | change | against | ratio | bar | |
|---|---:|---:|---:|---|---:|---:|---|
| plain | 1,923.0 | 1,922.0 | +0.05% | gnur | 0.401x | 0.6x | ok |
| defaults | 1,999.9 | 1,998.8 | +0.05% | gnur | 0.328x | 0.6x | ok |
| named | 1,849.1 | 1,848.1 | +0.05% | gnur | 0.399x | 0.6x | ok |
| env-fn | 2,507.6 | 2,506.6 | +0.04% | gnur | 0.654x | 1.0x | ok |
| superassign | 4,939.7 | 4,927.8 | +0.24% | gnur | 0.630x | 1.0x | ok |
| s3 | 18,750.3 | 19,304.6 | -2.87% | gnur | 0.976x | 1.0x | ok |
| fib | 10,722,697.4 | 10,722,567.4 | +0.00% | today | 1.272x | (1.0x) | not met yet |
| binarytrees | 3,475,944.2 | 3,474,932.0 | +0.03% | today | 0.840x | 1.0x | ok |
| e7-paste | 96,025.8 | 96,019.6 | +0.01% | today | 1.927x | (1.0x) | accepted up to 2.0x |
| e7-match | 99,022.6 | 99,008.1 | +0.01% | today | 0.865x | 1.0x | ok |
| e7-unique | 126,369.9 | 126,686.5 | -0.25% | today | 0.697x | 1.0x | ok |

| program | peak RSS KiB | main | bars |
|---|---:|---:|---|
| benchmarks/suite/binarytrees.R | 26,284 | 26,596 | ≤ today 32,156, gnur 42,060 |
| benchmarks/rt/mem/listgrow.R | 45,448 | 45,288 | ≤ gnur 54,160 |
| benchmarks/rt/mem/churn.R | 73,848 | 73,844 | ≤ gnur 134,392 |

(A bar in parentheses is not met yet: only regressions against main fail it, and an accepted regression's ceiling.)

0 failure(s); thresholds: 0.5% instructions, 2.0% peak RSS against main

# the same against main cc5d091e, at 754f0146 (before the merge)

| row | instructions | main | change | against | ratio | bar | |
|---|---:|---:|---:|---|---:|---:|---|
| plain | 1,920.0 | 1,920.0 | +0.00% | gnur | 0.400x | 0.6x | ok |
| defaults | 1,995.9 | 1,995.8 | +0.00% | gnur | 0.328x | 0.6x | ok |
| named | 1,846.2 | 1,846.1 | +0.00% | gnur | 0.398x | 0.6x | ok |
| env-fn | 2,508.6 | 2,507.5 | +0.04% | gnur | 0.655x | 1.0x | ok |
| superassign | 4,912.7 | 4,909.8 | +0.06% | gnur | 0.627x | 1.0x | ok |
| s3 | 18,749.4 | 19,304.5 | -2.88% | gnur | 0.976x | 1.0x | ok |
| fib | 10,936,504.1 | 10,951,948.0 | -0.14% | today | 1.298x | (1.0x) | not met yet |
| binarytrees | 3,474,196.0 | 3,474,446.0 | -0.01% | today | 0.839x | 1.0x | ok |
| e7-paste | 96,011.3 | 95,996.4 | +0.02% | today | 1.927x | (1.0x) | not met yet |
| e7-match | 99,200.7 | 99,208.3 | -0.01% | today | 0.867x | 1.0x | ok |
| e7-unique | 126,267.1 | 126,478.4 | -0.17% | today | 0.696x | 1.0x | ok |

| program | peak RSS KiB | main | bars |
|---|---:|---:|---|
| benchmarks/suite/binarytrees.R | 26,480 | 26,336 | ≤ today 32,156, gnur 42,060 |
| benchmarks/rt/mem/listgrow.R | 45,324 | 45,160 | ≤ gnur 54,160 |
| benchmarks/rt/mem/churn.R | 73,836 | 73,456 | ≤ gnur 134,392 |

(A bar in parentheses is not met yet: only regressions against main fail it.)

0 failure(s); thresholds: 0.5% instructions, 2.0% peak RSS against main

# the first build (3cb81b38), which failed the gate

| row | instructions | main | change | against | ratio | bar | |
|---|---:|---:|---:|---|---:|---:|---|
| plain | 1,938.1 | 1,920.1 | +0.94% | gnur | 0.404x | 0.6x | ok, SLOWER |
| defaults | 2,014.2 | 1,995.9 | +0.91% | gnur | 0.331x | 0.6x | ok, SLOWER |
| named | 1,859.2 | 1,846.1 | +0.70% | gnur | 0.401x | 0.6x | ok, SLOWER |
| env-fn | 2,523.6 | 2,507.6 | +0.64% | gnur | 0.659x | 1.0x | ok, SLOWER |
| superassign | 4,944.9 | 4,909.7 | +0.72% | gnur | 0.631x | 1.0x | ok, SLOWER |
| s3 | 18,793.5 | 19,304.5 | -2.65% | gnur | 0.978x | (1.0x) | ok |
| fib | 10,942,431.0 | 10,951,948.1 | -0.09% | today | 1.298x | (1.0x) | not met yet |
| binarytrees | 3,489,848.2 | 3,474,453.1 | +0.44% | today | 0.843x | 1.0x | ok |
| e7-paste | 96,103.4 | 95,993.9 | +0.11% | today | 1.929x | (1.0x) | not met yet |
| e7-match | 99,235.2 | 99,204.6 | +0.03% | today | 0.867x | 1.0x | ok |
| e7-unique | 126,175.0 | 126,477.7 | -0.24% | today | 0.696x | 1.0x | ok |

| program | peak RSS KiB | main | bars |
|---|---:|---:|---|
| benchmarks/suite/binarytrees.R | 26,340 | 26,412 | ≤ today 32,156, gnur 42,060 |
| benchmarks/rt/mem/listgrow.R | 45,404 | 45,160 | ≤ gnur 54,160 |
| benchmarks/rt/mem/churn.R | 73,932 | 73,664 | ≤ gnur 134,392 |

(A bar in parentheses is not met yet: only regressions against main fail it.)


FAIL `plain`: +0.94% instructions against main (1,920.1 -> 1,938.1)
