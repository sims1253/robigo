Reproduction, `main` (2 runs): spread max ÷ min − 1

| row | run 1 instr | run 1 est. cycles | run 2 instr | run 2 est. cycles | instr spread | cycles spread |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,010.0 | 2,024.9 | 2,009.9 | 2,024.8 | 0.004% | 0.002% |
| defaults | 2,073.9 | 2,089.2 | 2,073.9 | 2,089.0 | 0.003% | 0.007% |
| named | 1,932.6 | 1,947.4 | 1,932.6 | 1,947.5 | 0.000% | 0.002% |
| env-fn | 2,566.9 | 2,581.8 | 2,566.9 | 2,581.8 | 0.002% | 0.000% |
| superassign | 5,009.3 | 5,041.9 | 5,009.3 | 5,041.9 | 0.001% | 0.001% |
| s3 | 19,418.3 | 22,146.8 | 19,418.3 | 22,146.8 | 0.000% | 0.000% |
| fib | 12,027,681.3 | 12,135,315.3 | 12,027,696.0 | 12,135,331.5 | 0.000% | 0.000% |
| binarytrees | 3,719,298.2 | 3,891,811.2 | 3,719,297.9 | 3,891,808.1 | 0.000% | 0.000% |

Reproduction, `exact` (2 runs): spread max ÷ min − 1

| row | run 1 instr | run 1 est. cycles | run 2 instr | run 2 est. cycles | instr spread | cycles spread |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,009.9 | 2,024.7 | 2,009.9 | 2,024.7 | 0.003% | 0.003% |
| defaults | 2,073.8 | 2,088.7 | 2,073.9 | 2,088.7 | 0.001% | 0.000% |
| named | 1,932.6 | 1,947.4 | 1,932.5 | 1,947.4 | 0.003% | 0.001% |
| env-fn | 2,567.0 | 2,581.9 | 2,566.9 | 2,581.8 | 0.001% | 0.002% |
| superassign | 5,009.3 | 5,045.7 | 5,009.3 | 5,045.7 | 0.000% | 0.001% |
| s3 | 20,022.4 | 22,798.2 | 20,022.4 | 22,798.0 | 0.000% | 0.001% |
| fib | 12,028,142.8 | 12,128,127.8 | 12,028,137.6 | 12,128,123.1 | 0.000% | 0.000% |
| binarytrees | 3,720,855.2 | 3,878,221.5 | 3,720,852.3 | 3,878,220.5 | 0.000% | 0.000% |

Medians and `exact` ÷ `main`

| row | main instr | exact instr | ÷ | main est. cycles | exact est. cycles | ÷ |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,009.9 | 2,009.9 | 1.0000 | 2,024.9 | 2,024.7 | 0.9999 |
| defaults | 2,073.9 | 2,073.9 | 1.0000 | 2,089.1 | 2,088.7 | 0.9998 |
| named | 1,932.6 | 1,932.6 | 1.0000 | 1,947.5 | 1,947.4 | 1.0000 |
| env-fn | 2,566.9 | 2,566.9 | 1.0000 | 2,581.8 | 2,581.8 | 1.0000 |
| superassign | 5,009.3 | 5,009.3 | 1.0000 | 5,041.9 | 5,045.7 | 1.0008 |
| s3 | 19,418.3 | 20,022.4 | 1.0311 | 22,146.8 | 22,798.1 | 1.0294 |
| fib | 12,027,688.7 | 12,028,140.2 | 1.0000 | 12,135,323.4 | 12,128,125.4 | 0.9994 |
| binarytrees | 3,719,298.0 | 3,720,853.8 | 1.0004 | 3,891,809.7 | 3,878,221.0 | 0.9965 |
| geomean (plain, defaults, named, env-fn, superassign, s3, fib, binarytrees) | | | 1.0039 | | | 1.0032 |

Allocations per iteration (objects, bytes; by SEXPTYPE)

| row | main objects | main bytes | exact objects | exact bytes | main by type | exact by type |
|---|---:|---:|---:|---:|---|---|
| plain | 0.003 | 8.144 | 0.003 | 8.144 | REALSXP 0.003 | REALSXP 0.003 |
| defaults | 0.005 | 8.24 | 0.005 | 8.24 | REALSXP 0.005 | REALSXP 0.005 |
| named | 0 | 8 | 0 | 8 |  |  |
| env-fn | 0 | 8 | 0 | 8 |  |  |
| superassign | 1.02025 | 56.972 | 1.02025 | 56.972 | LISTSXP 0.00675, REALSXP 1.0135 | LISTSXP 0.00675, REALSXP 1.0135 |
| s3 | 16 | 840 | 17 | 888 | ENVSXP 2, LANGSXP 1, LISTSXP 7, PROMSXP 3, REALSXP 1, STRSXP 2 | ENVSXP 2, LANGSXP 1, LISTSXP 7, PROMSXP 4, REALSXP 1, STRSXP 2 |
| fib | 167.3 | 8,038.4 | 167.3 | 8,038.4 | REALSXP 167.3 | REALSXP 167.3 |
| binarytrees | 1,029.025 | 49,401.2 | 1,029.025 | 49,401.2 | LGLSXP 8.025, REALSXP 510, VECSXP 511 | LGLSXP 8.025, REALSXP 510, VECSXP 511 |
