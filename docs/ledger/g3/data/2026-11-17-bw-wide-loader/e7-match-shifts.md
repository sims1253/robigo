# e7-match under heap shifts (2026-11-17)

The row's script (`benchmarks/rt/calls-rows.toml`, N = 200 and 2N = 400)
with `zz <- paste0('z', 1:k)` before its setup, so the setup's 1,000
strings land elsewhere in the heap. Instructions per iteration from
`ROBIGO_COMPILE=1 scripts/callgrind-diff.py WORK BASE 200` (the gate's
N/2N difference). main: `fb429442`'s binary; BW: the branch at `cfbad96a`
(sha256 prefix 83f09160e966e174).

| k | main | BW |
|---:|---:|---:|
| 0 | 99,033 | 103,129 |
| 1 | 98,919 | 102,812 |
| 3 | 98,915 | 102,921 |
| 10 | 98,916 | 102,857 |
| 30 | 98,900 | 102,746 |
| 100 | 98,911 | 103,570 |
| 300 | 98,931 | 99,610 |
| 600 | 99,150 | 99,313 |
| 1000 | 99,276 | 98,907 |
| 1500 | 98,928 | 99,141 |
| 2000 | 99,224 | 99,292 |
| 3000 | 98,902 | 99,301 |
| 5000 | 99,263 | 98,940 |

The script for k = 3, N = 200:

```r
zz <- paste0('z', 1:3)
keys <- paste0("k", 1:1000)
q <- keys[seq(1L, 1000L, by = 10L)]

g <- function() { for (i in 1:200) { m <- match(q, keys) }; NULL }
g()
```
