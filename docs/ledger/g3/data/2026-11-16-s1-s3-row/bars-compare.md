Reproduction, `main` (2 runs): spread max ÷ min − 1

| row | run 1 instr | run 1 est. cycles | run 2 instr | run 2 est. cycles | instr spread | cycles spread |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,221.0 | 2,224.0 | 2,221.1 | 2,224.1 | 0.006% | 0.004% |
| defaults | 2,253.6 | 2,256.5 | 2,253.7 | 2,256.5 | 0.003% | 0.001% |
| named | 1,980.8 | 1,983.1 | 1,980.7 | 1,983.1 | 0.003% | 0.003% |
| env-fn | 2,586.1 | 2,588.8 | 2,586.1 | 2,588.7 | 0.002% | 0.005% |
| superassign | 8,836.3 | 9,433.6 | 8,836.2 | 9,433.5 | 0.001% | 0.001% |
| s3 | 32,336.2 | 37,786.7 | 32,336.5 | 37,776.8 | 0.001% | 0.026% |
| fib | 17,749,705.2 | 17,877,908.2 | 17,749,695.2 | 17,877,897.2 | 0.000% | 0.000% |
| binarytrees | 6,224,504.3 | 6,674,600.8 | 6,224,506.5 | 6,674,601.5 | 0.000% | 0.000% |

Reproduction, `s3row` (2 runs): spread max ÷ min − 1

| row | run 1 instr | run 1 est. cycles | run 2 instr | run 2 est. cycles | instr spread | cycles spread |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,221.0 | 2,224.0 | 2,221.1 | 2,224.1 | 0.005% | 0.004% |
| defaults | 2,253.6 | 2,256.7 | 2,253.7 | 2,256.9 | 0.003% | 0.007% |
| named | 1,980.8 | 1,983.2 | 1,980.7 | 1,983.0 | 0.005% | 0.007% |
| env-fn | 2,586.2 | 2,588.8 | 2,586.1 | 2,588.8 | 0.001% | 0.000% |
| superassign | 8,836.1 | 9,443.5 | 8,836.2 | 9,443.6 | 0.000% | 0.000% |
| s3 | 26,927.6 | 32,124.0 | 26,927.8 | 32,124.4 | 0.001% | 0.001% |
| fib | 17,749,677.8 | 17,889,409.8 | 17,749,667.4 | 17,889,394.4 | 0.000% | 0.000% |
| binarytrees | 6,224,500.8 | 6,685,495.5 | 6,224,505.8 | 6,685,505.6 | 0.000% | 0.000% |

Medians and `s3row` ÷ `main`

| row | main instr | s3row instr | ÷ | main est. cycles | s3row est. cycles | ÷ |
|---|---:|---:|---:|---:|---:|---:|
| plain | 2,221.1 | 2,221.1 | 1.0000 | 2,224.1 | 2,224.1 | 1.0000 |
| defaults | 2,253.6 | 2,253.6 | 1.0000 | 2,256.5 | 2,256.8 | 1.0001 |
| named | 1,980.8 | 1,980.7 | 1.0000 | 1,983.1 | 1,983.1 | 1.0000 |
| env-fn | 2,586.1 | 2,586.1 | 1.0000 | 2,588.8 | 2,588.8 | 1.0000 |
| superassign | 8,836.2 | 8,836.1 | 1.0000 | 9,433.5 | 9,443.5 | 1.0011 |
| s3 | 32,336.3 | 26,927.7 | 0.8327 | 37,781.7 | 32,124.2 | 0.8503 |
| fib | 17,749,700.2 | 17,749,672.6 | 1.0000 | 17,877,902.7 | 17,889,402.1 | 1.0006 |
| binarytrees | 6,224,505.4 | 6,224,503.3 | 1.0000 | 6,674,601.2 | 6,685,500.6 | 1.0016 |
| geomean (plain, defaults, named, env-fn, superassign, s3, fib, binarytrees) | | | 0.9774 | | | 0.9804 |

Allocations per iteration (objects, bytes; by SEXPTYPE)

| row | main objects | main bytes | s3row objects | s3row bytes | main by type | s3row by type |
|---|---:|---:|---:|---:|---|---|
| plain | 0.005 | 8.4 | 0.005 | 8.4 | ENVSXP 0.002, REALSXP 0.003 | ENVSXP 0.002, REALSXP 0.003 |
| defaults | 0.009 | 8.88 | 0.009 | 8.88 | ENVSXP 0.004, REALSXP 0.005 | ENVSXP 0.004, REALSXP 0.005 |
| named | 0.001 | 8.16 | 0.001 | 8.16 | ENVSXP 0.001 | ENVSXP 0.001 |
| env-fn | 0.001 | 8.096 | 0.001 | 8.096 | ENVSXP 0.001 | ENVSXP 0.001 |
| superassign | 4 | 248 | 4 | 248 | ENVSXP 1, LISTSXP 1, REALSXP 2 | ENVSXP 1, LISTSXP 1, REALSXP 2 |
| s3 | 29 | 1,992 | 27 | 1,896 | ENVSXP 5, LANGSXP 1, LISTSXP 14, PROMSXP 4, REALSXP 1, STRSXP 4 | ENVSXP 5, LANGSXP 1, LISTSXP 14, PROMSXP 4, REALSXP 1, STRSXP 2 |
| fib | 319.25 | 22,625.6 | 319.25 | 22,625.6 | ENVSXP 151.95, REALSXP 167.3 | ENVSXP 151.95, REALSXP 167.3 |
| binarytrees | 2,554 | 171,656 | 2,554 | 171,656 | ENVSXP 1,022, LGLSXP 511, REALSXP 510, VECSXP 511 | ENVSXP 1,022, LGLSXP 511, REALSXP 510, VECSXP 511 |
