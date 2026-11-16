Reproduction, `fix2` (2 runs): spread max ÷ min − 1

| row | run 1 instr | run 1 est. cycles | run 2 instr | run 2 est. cycles | instr spread | cycles spread |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,203.2 | 2,205.9 | 2,203.1 | 2,205.7 | 0.001% | 0.007% |
| defaults | 2,247.7 | 2,250.9 | 2,247.7 | 2,250.9 | 0.001% | 0.000% |
| named | 1,950.9 | 1,953.2 | 1,950.8 | 1,953.2 | 0.000% | 0.000% |
| env-fn | 2,568.2 | 2,570.7 | 2,568.1 | 2,570.6 | 0.005% | 0.003% |
| superassign | 5,141.1 | 5,298.4 | 5,140.9 | 5,298.2 | 0.003% | 0.004% |
| s3 | 21,695.1 | 24,824.1 | 21,695.2 | 24,824.2 | 0.000% | 0.000% |
| fib | 17,899,025.8 | 18,019,803.8 | 17,899,025.4 | 18,019,794.4 | 0.000% | 0.000% |
| binarytrees | 6,232,831.7 | 6,549,596.4 | 6,232,836.5 | 6,549,605.8 | 0.000% | 0.000% |

Reproduction, `fix4` (2 runs): spread max ÷ min − 1

| row | run 1 instr | run 1 est. cycles | run 2 instr | run 2 est. cycles | instr spread | cycles spread |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,203.2 | 2,218.3 | 2,203.3 | 2,218.5 | 0.008% | 0.007% |
| defaults | 2,247.7 | 2,263.1 | 2,247.7 | 2,263.1 | 0.000% | 0.002% |
| named | 1,950.6 | 1,965.5 | 1,951.0 | 1,965.8 | 0.020% | 0.020% |
| env-fn | 2,568.2 | 2,570.7 | 2,568.2 | 2,570.6 | 0.001% | 0.004% |
| superassign | 5,141.0 | 5,249.3 | 5,141.1 | 5,249.4 | 0.000% | 0.001% |
| s3 | 20,402.5 | 22,936.2 | 20,402.3 | 22,935.9 | 0.001% | 0.001% |
| fib | 17,899,332.4 | 18,009,722.9 | 17,899,342.4 | 18,009,722.9 | 0.000% | 0.000% |
| binarytrees | 6,235,390.1 | 6,526,127.1 | 6,235,389.4 | 6,526,117.4 | 0.000% | 0.000% |

Medians and `fix4` ÷ `fix2`

| row | fix2 instr | fix4 instr | ÷ | fix2 est. cycles | fix4 est. cycles | ÷ |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,203.1 | 2,203.2 | 1.0000 | 2,205.8 | 2,218.4 | 1.0057 |
| defaults | 2,247.7 | 2,247.7 | 1.0000 | 2,250.9 | 2,263.1 | 1.0054 |
| named | 1,950.9 | 1,950.8 | 1.0000 | 1,953.2 | 1,965.7 | 1.0064 |
| env-fn | 2,568.2 | 2,568.2 | 1.0000 | 2,570.6 | 2,570.6 | 1.0000 |
| superassign | 5,141.0 | 5,141.0 | 1.0000 | 5,298.3 | 5,249.3 | 0.9908 |
| s3 | 21,695.1 | 20,402.4 | 0.9404 | 24,824.2 | 22,936.0 | 0.9239 |
| fib | 17,899,025.6 | 17,899,337.4 | 1.0000 | 18,019,799.1 | 18,009,722.9 | 0.9994 |
| binarytrees | 6,232,834.1 | 6,235,389.8 | 1.0004 | 6,549,601.1 | 6,526,122.2 | 0.9964 |
| geomean (plain, defaults, named, env-fn, superassign, s3, fib, binarytrees) | | | 0.9924 | | | 0.9907 |

Allocations per iteration (objects, bytes; by SEXPTYPE)

| row | fix2 objects | fix2 bytes | fix4 objects | fix4 bytes | fix2 by type | fix4 by type |
|---|---:|---:|---:|---:|---|---|
| plain | 0.003 | 8.144 | 0.003 | 8.144 | REALSXP 0.003 | REALSXP 0.003 |
| defaults | 0.005 | 8.24 | 0.005 | 8.24 | REALSXP 0.005 | REALSXP 0.005 |
| named | 0 | 8 | 0 | 8 |  |  |
| env-fn | 0 | 8 | 0 | 8 |  |  |
| superassign | 1.02025 | 56.972 | 1.02025 | 56.972 | LISTSXP 0.00675, REALSXP 1.0135 | LISTSXP 0.00675, REALSXP 1.0135 |
| s3 | 18 | 936 | 16 | 840 | ENVSXP 2, LANGSXP 1, LISTSXP 8, PROMSXP 4, REALSXP 1, STRSXP 2 | ENVSXP 2, LANGSXP 1, LISTSXP 7, PROMSXP 3, REALSXP 1, STRSXP 2 |
| fib | 167.3 | 8,038.4 | 167.3 | 8,038.4 | REALSXP 167.3 | REALSXP 167.3 |
| binarytrees | 1,532 | 73,544 | 1,532 | 73,544 | LGLSXP 511, REALSXP 510, VECSXP 511 | LGLSXP 511, REALSXP 510, VECSXP 511 |
