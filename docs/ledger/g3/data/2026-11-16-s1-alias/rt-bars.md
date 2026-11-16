# perf.py rt-bars at 754f0146 against main cc5d091e (the gate's own run)

### robigo-rt's M1 bars (scripts/perf.py rt-bars)

Callgrind instructions per iteration; bars against docs/ledger/g3/2026-11-16-bars-oracle-baseline.md and today's robigo.
Against main: main's binary of the merge's base, on this runner.

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
