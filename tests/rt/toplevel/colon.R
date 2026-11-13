# `:` (seq.c): integer results when both ends fit, doubles otherwise,
# descending ranges, and the errors.
.Last.value
1:3; 3:1; -1:2; 1.5:4; 0.5:-2; TRUE:3L; 1e10:(1e10 + 2)
2147483646:2147483647
2147483647:2147483648
x <- 0; for (i in 1:5) x <- x + i; x
(1:30) * 1000000
1:0
Inf:Inf
