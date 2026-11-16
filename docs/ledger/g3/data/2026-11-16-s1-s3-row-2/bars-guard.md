Reproduction, `main` (2 runs): spread max ÷ min − 1

| row | run 1 instr | run 1 est. cycles | run 2 instr | run 2 est. cycles | instr spread | cycles spread |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,235.1 | 2,237.6 | 2,235.1 | 2,237.6 | 0.001% | 0.001% |
| defaults | 2,279.7 | 2,282.8 | 2,279.7 | 2,282.8 | 0.000% | 0.000% |
| named | 1,980.8 | 1,983.1 | 1,980.9 | 1,983.2 | 0.007% | 0.004% |
| env-fn | 2,596.1 | 2,603.3 | 2,596.2 | 2,603.3 | 0.003% | 0.001% |
| superassign | 5,155.5 | 5,264.8 | 5,155.5 | 5,264.8 | 0.000% | 0.000% |
| s3 | 26,976.5 | 32,308.4 | 26,976.7 | 32,308.6 | 0.001% | 0.001% |
| fib | 17,900,758.6 | 18,010,791.6 | 17,900,723.6 | 18,010,754.1 | 0.000% | 0.000% |
| binarytrees | 6,234,615.4 | 6,497,577.2 | 6,234,619.5 | 6,497,578.0 | 0.000% | 0.000% |

Reproduction, `guard` (2 runs): spread max ÷ min − 1

| row | run 1 instr | run 1 est. cycles | run 2 instr | run 2 est. cycles | instr spread | cycles spread |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,203.1 | 2,205.7 | 2,203.1 | 2,205.8 | 0.001% | 0.003% |
| defaults | 2,247.6 | 2,250.9 | 2,247.7 | 2,250.9 | 0.002% | 0.002% |
| named | 1,950.7 | 1,953.2 | 1,950.7 | 1,953.2 | 0.001% | 0.001% |
| env-fn | 2,568.2 | 2,575.3 | 2,568.0 | 2,575.1 | 0.008% | 0.007% |
| superassign | 5,141.5 | 5,307.8 | 5,141.6 | 5,308.0 | 0.002% | 0.002% |
| s3 | 26,948.5 | 32,319.2 | 26,948.7 | 32,319.5 | 0.001% | 0.001% |
| fib | 17,897,659.4 | 18,007,362.4 | 17,897,612.4 | 18,007,316.9 | 0.000% | 0.000% |
| binarytrees | 6,221,571.5 | 6,499,542.5 | 6,221,581.7 | 6,499,553.2 | 0.000% | 0.000% |

Medians and `guard` ÷ `main`

| row | main instr | guard instr | ÷ | main est. cycles | guard est. cycles | ÷ |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,235.1 | 2,203.1 | 0.9857 | 2,237.6 | 2,205.8 | 0.9858 |
| defaults | 2,279.7 | 2,247.7 | 0.9859 | 2,282.8 | 2,250.9 | 0.9860 |
| named | 1,980.8 | 1,950.7 | 0.9848 | 1,983.2 | 1,953.2 | 0.9849 |
| env-fn | 2,596.2 | 2,568.1 | 0.9892 | 2,603.3 | 2,575.2 | 0.9892 |
| superassign | 5,155.5 | 5,141.5 | 0.9973 | 5,264.8 | 5,307.9 | 1.0082 |
| s3 | 26,976.6 | 26,948.6 | 0.9990 | 32,308.5 | 32,319.3 | 1.0003 |
| fib | 17,900,741.1 | 17,897,635.9 | 0.9998 | 18,010,772.8 | 18,007,339.6 | 0.9998 |
| binarytrees | 6,234,617.5 | 6,221,576.6 | 0.9979 | 6,497,577.6 | 6,499,547.8 | 1.0003 |
| geomean (plain, defaults, named, env-fn, superassign, s3, fib, binarytrees) | | | 0.9924 | | | 0.9943 |

Allocations per iteration (objects, bytes; by SEXPTYPE)

| row | main objects | main bytes | guard objects | guard bytes | main by type | guard by type |
|---|---:|---:|---:|---:|---|---|
| plain | 0.003 | 8.144 | 0.003 | 8.144 | REALSXP 0.003 | REALSXP 0.003 |
| defaults | 0.005 | 8.24 | 0.005 | 8.24 | REALSXP 0.005 | REALSXP 0.005 |
| named | 0 | 8 | 0 | 8 |  |  |
| env-fn | 0 | 8 | 0 | 8 |  |  |
| superassign | 1.02025 | 56.972 | 1.02025 | 56.972 | LISTSXP 0.00675, REALSXP 1.0135 | LISTSXP 0.00675, REALSXP 1.0135 |
| s3 | 27 | 1,896 | 27 | 1,896 | ENVSXP 5, LANGSXP 1, LISTSXP 14, PROMSXP 4, REALSXP 1, STRSXP 2 | ENVSXP 5, LANGSXP 1, LISTSXP 14, PROMSXP 4, REALSXP 1, STRSXP 2 |
| fib | 167.3 | 8,038.4 | 167.3 | 8,038.4 | REALSXP 167.3 | REALSXP 167.3 |
| binarytrees | 1,532 | 73,544 | 1,532 | 73,544 | LGLSXP 511, REALSXP 510, VECSXP 511 | LGLSXP 511, REALSXP 510, VECSXP 511 |
