Reproduction, `exact` (2 runs): spread max ÷ min − 1

| row | run 1 instr | run 1 est. cycles | run 2 instr | run 2 est. cycles | instr spread | cycles spread |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,009.9 | 2,024.7 | 2,009.9 | 2,024.8 | 0.002% | 0.002% |
| defaults | 2,073.9 | 2,088.8 | 2,073.9 | 2,088.8 | 0.001% | 0.001% |
| named | 1,932.6 | 1,947.3 | 1,932.6 | 1,947.4 | 0.001% | 0.003% |
| env-fn | 2,566.9 | 2,581.8 | 2,567.0 | 2,581.9 | 0.004% | 0.005% |
| superassign | 5,009.3 | 5,045.7 | 5,009.3 | 5,045.6 | 0.000% | 0.001% |
| s3 | 20,022.2 | 22,798.0 | 20,022.1 | 22,797.8 | 0.000% | 0.001% |
| fib | 12,028,144.2 | 12,128,119.7 | 12,028,151.7 | 12,128,141.7 | 0.000% | 0.000% |
| binarytrees | 3,720,853.1 | 3,878,221.3 | 3,720,854.9 | 3,878,215.4 | 0.000% | 0.000% |

Reproduction, `plans` (2 runs): spread max ÷ min − 1

| row | run 1 instr | run 1 est. cycles | run 2 instr | run 2 est. cycles | instr spread | cycles spread |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,010.0 | 2,024.8 | 2,009.9 | 2,024.7 | 0.003% | 0.000% |
| defaults | 2,073.9 | 2,088.9 | 2,073.9 | 2,088.9 | 0.001% | 0.002% |
| named | 1,932.6 | 1,947.4 | 1,932.5 | 1,947.3 | 0.004% | 0.005% |
| env-fn | 2,567.0 | 2,581.9 | 2,566.9 | 2,581.8 | 0.003% | 0.002% |
| superassign | 5,009.3 | 5,128.8 | 5,009.4 | 5,128.9 | 0.002% | 0.002% |
| s3 | 19,797.5 | 22,674.6 | 19,797.4 | 22,674.3 | 0.001% | 0.001% |
| fib | 12,028,154.4 | 12,128,836.9 | 12,028,151.2 | 12,128,832.2 | 0.000% | 0.000% |
| binarytrees | 3,720,855.5 | 3,891,296.5 | 3,720,859.0 | 3,891,298.0 | 0.000% | 0.000% |

Medians and `plans` ÷ `exact`

| row | exact instr | plans instr | ÷ | exact est. cycles | plans est. cycles | ÷ |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,009.9 | 2,009.9 | 1.0000 | 2,024.8 | 2,024.8 | 1.0000 |
| defaults | 2,073.9 | 2,073.9 | 1.0000 | 2,088.8 | 2,088.9 | 1.0001 |
| named | 1,932.6 | 1,932.5 | 1.0000 | 1,947.3 | 1,947.3 | 1.0000 |
| env-fn | 2,567.0 | 2,566.9 | 1.0000 | 2,581.9 | 2,581.8 | 1.0000 |
| superassign | 5,009.3 | 5,009.3 | 1.0000 | 5,045.7 | 5,128.9 | 1.0165 |
| s3 | 20,022.2 | 19,797.4 | 0.9888 | 22,797.9 | 22,674.4 | 0.9946 |
| fib | 12,028,147.9 | 12,028,152.8 | 1.0000 | 12,128,130.7 | 12,128,834.6 | 1.0001 |
| binarytrees | 3,720,854.0 | 3,720,857.2 | 1.0000 | 3,878,218.3 | 3,891,297.2 | 1.0034 |
| geomean (plain, defaults, named, env-fn, superassign, s3, fib, binarytrees) | | | 0.9986 | | | 1.0018 |

Allocations per iteration (objects, bytes; by SEXPTYPE)

| row | exact objects | exact bytes | plans objects | plans bytes | exact by type | plans by type |
|---|---:|---:|---:|---:|---|---|
| plain | 0.003 | 8.144 | 0.003 | 8.144 | REALSXP 0.003 | REALSXP 0.003 |
| defaults | 0.005 | 8.24 | 0.005 | 8.24 | REALSXP 0.005 | REALSXP 0.005 |
| named | 0 | 8 | 0 | 8 |  |  |
| env-fn | 0 | 8 | 0 | 8 |  |  |
| superassign | 1.02025 | 56.972 | 1.02025 | 56.972 | LISTSXP 0.00675, REALSXP 1.0135 | LISTSXP 0.00675, REALSXP 1.0135 |
| s3 | 17 | 888 | 17 | 888 | ENVSXP 2, LANGSXP 1, LISTSXP 7, PROMSXP 4, REALSXP 1, STRSXP 2 | ENVSXP 2, LANGSXP 1, LISTSXP 7, PROMSXP 4, REALSXP 1, STRSXP 2 |
| fib | 167.3 | 8,038.4 | 167.3 | 8,038.4 | REALSXP 167.3 | REALSXP 167.3 |
| binarytrees | 1,029.025 | 49,401.2 | 1,029.025 | 49,401.2 | LGLSXP 8.025, REALSXP 510, VECSXP 511 | LGLSXP 8.025, REALSXP 510, VECSXP 511 |
