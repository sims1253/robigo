# RNGkind: the kinds before the change, each new kind seeded from the
# old generator's next uniform
set.seed(1)
RNGkind()
old <- RNGkind("Wichmann-Hill")
old
RNGkind()
.Random.seed
RNGkind("Super")
.Random.seed
RNGkind(normal.kind = "Kinderman-Ramage")
.Random.seed[1]
RNGkind(sample.kind = "Rounding")
.Random.seed[1]
RNGkind("default", "default", "default")
RNGkind()
length(.Random.seed)
.Random.seed[1:5]
RNGkind("L'Ecuyer")
.Random.seed
RNGkind("Knuth-TAOCP-2002")
length(.Random.seed)
.Random.seed[c(1:4, 100:102)]
RNGkind("Knuth-TAOCP")
.Random.seed[c(1:4, 100:102)]
RNGkind("Mersenne")
.Random.seed[1:3]
# without a .Random.seed, RNGkind() seeds from the time (GetRNGstate)
# and writes none
rm(.Random.seed)
RNGkind()
exists(".Random.seed")
