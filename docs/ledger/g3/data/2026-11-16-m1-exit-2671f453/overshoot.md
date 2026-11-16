rt-overshoot: robigo-rt, 304 programs x 2 profiles

| program | profile | collections | max overshoot | live after | bound | max / bound | over |
|---|---|---:|---:|---:|---:|---:|---:|
| benchmarks/rt/mem/churn.R | eval | 79 | 8,000,032 | 20,926,080 | 33,554,432 | 0.2384 | 0 |
| benchmarks/rt/mem/churn.R | bc | 79 | 8,000,032 | 24,924,864 | 33,554,432 | 0.2384 | 0 |
| tests/rt/capi/weak-keys-collections.R | eval | 120 | 800,032 | 755,056 | 33,554,432 | 0.0238 | 0 |
| tests/rt/capi/weak-keys-collections.R | bc | 120 | 800,032 | 750,544 | 33,554,432 | 0.0238 | 0 |
| tests/rt/stress/finalizer-depths.R | eval | 2113 | 800,032 | 771,168 | 33,554,432 | 0.0238 | 0 |
| tests/rt/stress/finalizer-depths.R | bc | 2113 | 800,032 | 762,080 | 33,554,432 | 0.0238 | 0 |
| tests/rt/sort/radix-volume.R | bc | 5 | 560,032 | 5,236,976 | 33,554,432 | 0.0167 | 0 |
| tests/rt/sort/radix-volume.R | eval | 7 | 280,032 | 4,739,024 | 33,554,432 | 0.0083 | 0 |
| tests/rt/capi/pin_premature.R | eval | 53 | 1,120 | 751,376 | 33,554,432 | 0.0000 | 0 |
| tests/rt/capi/pin_premature.R | bc | 51 | 976 | 744,352 | 33,554,432 | 0.0000 | 0 |
| benchmarks/rt/mem/listgrow.R | eval | 3 | 144 | 2,113,520 | 33,554,432 | 0.0000 | 0 |
| benchmarks/suite/calls.R | bc | 2 | 96 | 858,368 | 33,554,432 | 0.0000 | 0 |
| benchmarks/suite/calls.R | eval | 3 | 48 | 892,672 | 33,554,432 | 0.0000 | 0 |
| benchmarks/suite/binarytrees.R | eval | 2 | 48 | 940,640 | 33,554,432 | 0.0000 | 0 |
| benchmarks/suite/binarytrees.R | bc | 1 | 48 | 840,416 | 33,554,432 | 0.0000 | 0 |

runs: 608; with a report: 608; over the bound: 0; a collection due at exit: 0; largest max/bound: 0.2384
::endgroup::
