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

Medians and `fix2` ÷ `main`

| row | main instr | fix2 instr | ÷ | main est. cycles | fix2 est. cycles | ÷ |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,235.1 | 2,203.1 | 0.9857 | 2,237.6 | 2,205.8 | 0.9858 |
| defaults | 2,279.7 | 2,247.7 | 0.9859 | 2,282.8 | 2,250.9 | 0.9860 |
| named | 1,980.8 | 1,950.9 | 0.9849 | 1,983.2 | 1,953.2 | 0.9849 |
| env-fn | 2,596.2 | 2,568.2 | 0.9892 | 2,603.3 | 2,570.6 | 0.9874 |
| superassign | 5,155.5 | 5,141.0 | 0.9972 | 5,264.8 | 5,298.3 | 1.0064 |
| s3 | 26,976.6 | 21,695.1 | 0.8042 | 32,308.5 | 24,824.2 | 0.7683 |
| fib | 17,900,741.1 | 17,899,025.6 | 0.9999 | 18,010,772.8 | 18,019,799.1 | 1.0005 |
| binarytrees | 6,234,617.5 | 6,232,834.1 | 0.9997 | 6,497,577.6 | 6,549,601.1 | 1.0080 |
| geomean (plain, defaults, named, env-fn, superassign, s3, fib, binarytrees) | | | 0.9661 | | | 0.9626 |

Allocations per iteration (objects, bytes; by SEXPTYPE)

| row | main objects | main bytes | fix2 objects | fix2 bytes | main by type | fix2 by type |
|---|---:|---:|---:|---:|---|---|
| plain | 0.003 | 8.144 | 0.003 | 8.144 | REALSXP 0.003 | REALSXP 0.003 |
| defaults | 0.005 | 8.24 | 0.005 | 8.24 | REALSXP 0.005 | REALSXP 0.005 |
| named | 0 | 8 | 0 | 8 |  |  |
| env-fn | 0 | 8 | 0 | 8 |  |  |
| superassign | 1.02025 | 56.972 | 1.02025 | 56.972 | LISTSXP 0.00675, REALSXP 1.0135 | LISTSXP 0.00675, REALSXP 1.0135 |
| s3 | 27 | 1,896 | 18 | 936 | ENVSXP 5, LANGSXP 1, LISTSXP 14, PROMSXP 4, REALSXP 1, STRSXP 2 | ENVSXP 2, LANGSXP 1, LISTSXP 8, PROMSXP 4, REALSXP 1, STRSXP 2 |
| fib | 167.3 | 8,038.4 | 167.3 | 8,038.4 | REALSXP 167.3 | REALSXP 167.3 |
| binarytrees | 1,532 | 73,544 | 1,532 | 73,544 | LGLSXP 511, REALSXP 510, VECSXP 511 | LGLSXP 511, REALSXP 510, VECSXP 511 |
