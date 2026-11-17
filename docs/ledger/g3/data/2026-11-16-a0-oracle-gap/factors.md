# A0's factors, every row

Each factor is the ratio of two cells' instructions per iteration (`instr.json`, the mean of two runs),
as `../../2026-11-16-a0-oracle-gap.md` describes.

## Options: Debian's r-base set against A0's: oracle ÷ img-deb

| row | plain | defaults | named | env-fn | superassign | s3 | trycatch | fib | binarytrees | e7-paste | e7-match | e7-unique | geomean |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ratio | +0.0% | +0.0% | +0.0% | -0.0% | +0.0% | -0.1% | +0.0% | -0.0% | -0.0% | -0.0% | -0.6% | +5.2% | +0.4% |

## Debian's flags, image toolchain: img-deb ÷ img-plain

| row | plain | defaults | named | env-fn | superassign | s3 | trycatch | fib | binarytrees | e7-paste | e7-match | e7-unique | geomean |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ratio | +2.3% | +2.4% | +2.0% | +2.5% | +3.2% | +2.5% | +2.2% | +2.2% | +2.1% | +4.7% | +1.2% | +1.6% | +2.4% |

## Debian's flags, host toolchain: host-deb ÷ host-plain

| row | plain | defaults | named | env-fn | superassign | s3 | trycatch | fib | binarytrees | e7-paste | e7-match | e7-unique | geomean |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ratio | +2.6% | +2.6% | +2.7% | +2.5% | +2.9% | +2.5% | +2.7% | +2.5% | +2.6% | +5.2% | +2.1% | +1.7% | +2.7% |

## Toolchain, with Debian's flags: img-deb ÷ host-deb

| row | plain | defaults | named | env-fn | superassign | s3 | trycatch | fib | binarytrees | e7-paste | e7-match | e7-unique | geomean |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ratio | +4.5% | +5.1% | +3.2% | +3.3% | +1.8% | +1.2% | +3.3% | +3.2% | +3.1% | +4.9% | +6.0% | +5.2% | +3.7% |

## Toolchain, plain flags: img-plain ÷ host-plain

| row | plain | defaults | named | env-fn | superassign | s3 | trycatch | fib | binarytrees | e7-paste | e7-match | e7-unique | geomean |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ratio | +4.9% | +5.3% | +3.9% | +3.3% | +1.5% | +1.2% | +3.8% | +3.6% | +3.7% | +5.4% | +6.9% | +5.4% | +4.1% |

## Memory profiling: host-plain ÷ host-local

| row | plain | defaults | named | env-fn | superassign | s3 | trycatch | fib | binarytrees | e7-paste | e7-match | e7-unique | geomean |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ratio | +0.0% | +0.0% | +0.0% | +0.0% | +0.1% | +0.1% | +0.0% | +0.0% | +0.0% | +0.1% | +0.0% | +0.0% | +0.0% |

## The whole gap: oracle ÷ host-local

| row | plain | defaults | named | env-fn | superassign | s3 | trycatch | fib | binarytrees | e7-paste | e7-match | e7-unique | geomean |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ratio | +7.3% | +7.9% | +6.0% | +5.9% | +4.8% | +3.7% | +6.1% | +5.9% | +5.9% | +10.4% | +7.6% | +12.6% | +7.0% |

## Shared libR against GNU R's static default: img-plain ÷ img-stock

| row | plain | defaults | named | env-fn | superassign | s3 | trycatch | fib | binarytrees | e7-paste | e7-match | e7-unique | geomean |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ratio | +9.8% | +9.1% | +8.6% | +10.3% | +10.2% | +7.7% | +8.1% | +10.7% | +9.7% | +5.7% | +0.6% | +0.9% | +7.6% |

## The oracle image against F2 (A)'s rival: oracle ÷ img-stock

| row | plain | defaults | named | env-fn | superassign | s3 | trycatch | fib | binarytrees | e7-paste | e7-match | e7-unique | geomean |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ratio | +12.3% | +11.7% | +10.8% | +13.1% | +13.7% | +10.3% | +10.6% | +13.2% | +12.1% | +10.6% | +1.2% | +7.8% | +10.6% |

