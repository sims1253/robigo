# set.seed and .Random.seed for every generator (the seed is the state:
# no runif in base)
set.seed(42)
length(.Random.seed)
.Random.seed[1:6]
.Random.seed[620:626]
set.seed(-1L)
.Random.seed[1:4]
set.seed(2^31 - 1)
.Random.seed[1:4]
for (k in c("Wichmann-Hill", "Marsaglia-Multicarry", "Super-Duper", "Mersenne-Twister",
            "Knuth-TAOCP", "Knuth-TAOCP-2002", "L'Ecuyer-CMRG")) {
    suppressWarnings(set.seed(123, kind = k))
    n <- length(.Random.seed)
    cat(k, n, .Random.seed[1:min(n, 8)], .Random.seed[(n - 1):n], "\n")
}
set.seed(1, kind = "default", normal.kind = "default", sample.kind = "default")
RNGkind()
.Random.seed[1:3]
set.seed(7, normal.kind = "Box-Muller")
RNGkind()
.Random.seed[1:3]
set.seed(7, sample.kind = "Rounding")
RNGkind()
.Random.seed[1]
set.seed(7, kind = "Wichmann-Hill", normal.kind = "Ahrens", sample.kind = "Rej")
RNGkind()
.Random.seed
set.seed(3.9)
.Random.seed[2:4]
set.seed("12")
.Random.seed[2:4]
set.seed(TRUE)
.Random.seed[2:4]
