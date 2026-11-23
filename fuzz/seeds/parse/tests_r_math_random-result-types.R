library(stats)
set.seed(123)
for (f in list(function(n) rpois(n, 2), function(n) rbinom(n, 10, .3),
               function(n) rgeom(n, .5), function(n) rnbinom(n, 3, .5),
               function(n) rhyper(n, 4, 6, 3), function(n) rsignrank(n, 4),
               function(n) rwilcox(n, 3, 4))) {
    print(typeof(f(0)))
    print(typeof(f(4)))
}
print(rpois(3, c(NA, -1, 0)))
print(rbinom(3, numeric(), .5))
print(typeof(rbinom(3, 3e9, 1)))
print(rbinom(3, c(NA, 3e9, 1), 1))
print(rbinom(3, c(1, 1, 3e9), 1))
print(typeof(rnbinom(0, 2, mu=4)))
print(tryCatch(rwilcox(1, 2147483648, 1), error=function(e) conditionMessage(e)))
print(rsignrank(2, Inf))
