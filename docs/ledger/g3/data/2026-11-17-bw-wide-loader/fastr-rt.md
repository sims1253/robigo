# fastr on robigo-rt with BW (2026-11-17)

A1's `scripts/corpus.py run fastr --runtime rt --jobs 4` from `g3/rc-rt-corpora` at `61ce3eec`, with the BW binary built at `0236869f` (sha256 prefix 2c79a337599f60aa), against that branch's `passing-rt/` (18,812 cases at main `fb429442`).

```text
fastr on robigo-rt: 21696/31374 pass; stdout 23181, stderr 21776, status 23195 match; 2884 new, 0 regressions
  most cases blocked by (the first missing thing each failing case hit):
    1,251  primitive ceiling
      501  not implemented: printing a matrix (printMatrix)
      392  .Internal set.seed
      324  not implemented: options(keep.source = )
      236  .Internal array
      203  unbound deriv
      197  primitive crossprod
      179  not implemented: loadNamespace
      137  .Internal cbind
      137  not implemented: '[<-' with matrix or array subscripts
      122  .Internal body
      121  primitive log
      116  .Internal format
       97  .Internal file
       96  .Internal rbind
       93  primitive %*%
       79  .Internal textConnection
       79  unbound qbeta
       72  .Internal t.default
       69  unbound pbeta
       67  .Internal gsub
       67  .Internal strsplit
       64  unbound setClass
       63  .Internal serialize
       59  .Internal regexpr
  (2884 new passes; --update records them)
```
