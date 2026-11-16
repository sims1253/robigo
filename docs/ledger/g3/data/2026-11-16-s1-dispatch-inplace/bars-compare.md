Reproduction, `main` (2 runs): spread max ÷ min − 1

| row | run 1 instr | run 1 est. cycles | run 2 instr | run 2 est. cycles | instr spread | cycles spread |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,014.5 | 2,029.3 | 2,014.4 | 2,029.2 | 0.003% | 0.003% |
| defaults | 2,079.2 | 2,094.1 | 2,079.2 | 2,094.1 | 0.002% | 0.002% |
| named | 1,936.9 | 1,951.6 | 1,936.8 | 1,951.6 | 0.001% | 0.001% |
| env-fn | 2,579.2 | 2,594.1 | 2,579.1 | 2,594.1 | 0.003% | 0.003% |
| superassign | 5,029.3 | 5,060.7 | 5,029.4 | 5,060.6 | 0.001% | 0.001% |
| s3 | 20,287.4 | 23,059.5 | 20,287.4 | 23,059.6 | 0.000% | 0.000% |
| fib | 12,056,837.2 | 12,159,638.2 | 12,056,835.0 | 12,159,645.5 | 0.000% | 0.000% |
| binarytrees | 3,829,824.2 | 3,985,077.0 | 3,829,816.8 | 3,985,064.8 | 0.000% | 0.000% |

Reproduction, `exact` (2 runs): spread max ÷ min − 1

| row | run 1 instr | run 1 est. cycles | run 2 instr | run 2 est. cycles | instr spread | cycles spread |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,014.4 | 2,029.2 | 2,014.4 | 2,029.2 | 0.001% | 0.002% |
| defaults | 2,079.1 | 2,094.0 | 2,079.1 | 2,094.0 | 0.001% | 0.000% |
| named | 1,936.8 | 1,951.6 | 1,936.8 | 1,951.6 | 0.001% | 0.002% |
| env-fn | 2,579.1 | 2,593.9 | 2,579.2 | 2,594.1 | 0.003% | 0.004% |
| superassign | 5,029.1 | 5,063.0 | 5,029.2 | 5,063.0 | 0.001% | 0.001% |
| s3 | 20,838.5 | 23,724.6 | 20,838.4 | 23,724.6 | 0.000% | 0.000% |
| fib | 12,052,590.0 | 12,149,553.0 | 12,052,585.6 | 12,149,558.6 | 0.000% | 0.000% |
| binarytrees | 3,815,296.1 | 3,962,413.1 | 3,815,286.0 | 3,962,391.5 | 0.000% | 0.001% |

Medians and `exact` ÷ `main`

| row | main instr | exact instr | ÷ | main est. cycles | exact est. cycles | ÷ |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,014.5 | 2,014.4 | 1.0000 | 2,029.3 | 2,029.2 | 1.0000 |
| defaults | 2,079.2 | 2,079.1 | 1.0000 | 2,094.1 | 2,094.0 | 1.0000 |
| named | 1,936.9 | 1,936.8 | 1.0000 | 1,951.6 | 1,951.6 | 1.0000 |
| env-fn | 2,579.2 | 2,579.1 | 1.0000 | 2,594.1 | 2,594.0 | 1.0000 |
| superassign | 5,029.3 | 5,029.1 | 1.0000 | 5,060.6 | 5,063.0 | 1.0005 |
| s3 | 20,287.4 | 20,838.4 | 1.0272 | 23,059.5 | 23,724.6 | 1.0288 |
| fib | 12,056,836.1 | 12,052,587.8 | 0.9996 | 12,159,641.8 | 12,149,555.8 | 0.9992 |
| binarytrees | 3,829,820.5 | 3,815,291.1 | 0.9962 | 3,985,070.9 | 3,962,402.3 | 0.9943 |
| geomean (plain, defaults, named, env-fn, superassign, s3, fib, binarytrees) | | | 1.0028 | | | 1.0028 |

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
