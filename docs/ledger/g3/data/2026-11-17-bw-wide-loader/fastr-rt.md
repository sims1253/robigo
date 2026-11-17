# fastr on robigo-rt with BW (2026-11-17)

`scripts/corpus.py run fastr --runtime rt --jobs 4 --update` on the branch merged with main `908cb765` (RX, #764; BW `cc3854f6`), against the ratchet of the earlier merges. The first BW binary passed 21,696; merged with main `6bfbf91f` (L1), 21,808; this run adds 326, so 22,134 against main's 18,812.

```text
fastr on robigo-rt: 22134/31374 pass; stdout 23535, stderr 22215, status 23551 match; 326 new, 0 regressions
  most cases blocked by (the first missing thing each failing case hit):
    1,251  primitive ceiling
      519  not implemented: printing a matrix (printMatrix)
      392  .Internal set.seed
      324  not implemented: options(keep.source = )
      236  .Internal array
      203  unbound deriv
      197  primitive crossprod
      179  not implemented: loadNamespace
      150  .Internal cbind
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
       64  unbound setClass
       59  .Internal stdin
       57  unbound type.convert
       55  .Internal make.names
       54  not implemented: '[' with array subscripts
       52  primitive range
ratcheted 326 case(s) under tests/corpora/fastr/passing-rt/
```
