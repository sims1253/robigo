::group::overshoot on every slice (E4, scripts/rt-overshoot.py)
rt-overshoot: /home/m0hawk/.cache/cargo-target-m1/release/robigo-rt, 314 programs x 2 profiles

| program | profile | collections | max overshoot | live after | bound | max / bound | over |
|---|---|---:|---:|---:|---:|---:|---:|
| benchmarks/rt/mem/churn.R | eval | 79 | 8,000,032 | 20,938,320 | 33,554,432 | 0.2384 | 0 |
| benchmarks/rt/mem/churn.R | bc | 79 | 8,000,032 | 24,937,008 | 33,554,432 | 0.2384 | 0 |
| tests/rt/capi/weak-keys-collections.R | eval | 120 | 800,032 | 767,296 | 33,554,432 | 0.0238 | 0 |
| tests/rt/capi/weak-keys-collections.R | bc | 120 | 800,032 | 762,688 | 33,554,432 | 0.0238 | 0 |
| tests/rt/stress/finalizer-depths.R | eval | 2113 | 800,032 | 783,408 | 33,554,432 | 0.0238 | 0 |
| tests/rt/stress/finalizer-depths.R | bc | 2113 | 800,032 | 774,224 | 33,554,432 | 0.0238 | 0 |
| tests/rt/sort/radix-volume.R | bc | 5 | 560,032 | 5,249,168 | 33,554,432 | 0.0167 | 0 |
| tests/rt/sort/radix-volume.R | eval | 7 | 280,032 | 4,751,216 | 33,554,432 | 0.0083 | 0 |
| tests/rt/capi/pin_premature.R | eval | 53 | 1,120 | 763,616 | 33,554,432 | 0.0000 | 0 |
| benchmarks/suite/calls.R | eval | 3 | 1,040 | 908,816 | 33,554,432 | 0.0000 | 0 |
| tests/rt/capi/pin_premature.R | bc | 51 | 976 | 756,592 | 33,554,432 | 0.0000 | 0 |
| benchmarks/rt/mem/listgrow.R | eval | 3 | 240 | 2,125,760 | 33,554,432 | 0.0000 | 0 |
| benchmarks/suite/calls.R | bc | 2 | 96 | 872,384 | 33,554,432 | 0.0000 | 0 |
| benchmarks/suite/binarytrees.R | eval | 2 | 48 | 956,768 | 33,554,432 | 0.0000 | 0 |
| benchmarks/suite/binarytrees.R | bc | 1 | 48 | 846,944 | 33,554,432 | 0.0000 | 0 |

runs: 628; with a report: 628; over the bound: 0; a collection due at exit: 0; largest max/bound: 0.2384
::endgroup::
