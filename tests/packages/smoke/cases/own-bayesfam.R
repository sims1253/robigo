# needs: bayesfam
# The user's own package (github.com/sims1253/bayesfam): custom brms
# families. Its README fits a brms model on rbetaprime() draws; this
# keeps the pure-R part (the RNG, density and quantile helpers) and
# fits nothing.
suppressPackageStartupMessages(library(bayesfam))
set.seed(1)
print(rbetaprime(5, mu = exp(2.3), phi = 2))
x <- seq(0.05, 0.95, length.out = 5)
print(dbeta_mean(x, mu = 0.4, phi = 5))
print(qbeta_mean(c(0.1, 0.5, 0.9), mu = 0.4, phi = 5))
print(rbeta_mean(5, mu = 0.4, phi = 5))
