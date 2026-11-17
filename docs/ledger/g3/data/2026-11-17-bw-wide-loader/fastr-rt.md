# fastr on robigo-rt with BW (2026-11-17)

`scripts/corpus.py run fastr --runtime rt --jobs 4 --update` on the branch merged with main `6bfbf91f` (BW `d6148372`, sha256 prefix 7c2f1c35158d2905), against RC's `passing-rt/` (#755). Before this run, the merge with main `c4a60e89` ratcheted 2,884 new passes (21,696). This run adds 112, which come from L1's serialization (#752) together with the wide base.

```text
fastr on robigo-rt: 21808/31374 pass; stdout 23241, stderr 21889, status 23256 match; 112 new, 0 regressions
  most cases blocked by (the first missing thing each failing case hit):
    1,251  primitive ceiling
      501  not implemented: printing a matrix (printMatrix)
      392  .Internal set.seed
      324  not implemented: options(keep.source = )
      236  .Internal array
      203  unbound deriv
      197  primitive crossprod
      179  not implemented: loadNamespace
      138  .Internal cbind
      137  not implemented: '[<-' with matrix or array subscripts
      122  .Internal body
      121  primitive log
      116  .Internal format
       96  .Internal rbind
       93  primitive %*%
       79  .Internal textConnection
       79  unbound qbeta
       72  .Internal t.default
       69  unbound pbeta
       67  .Internal gsub
       67  .Internal strsplit
       64  unbound setClass
       59  .Internal regexpr
       59  .Internal stdin
       57  unbound type.convert
ratcheted 112 case(s) under tests/corpora/fastr/passing-rt/
```
