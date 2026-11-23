# The boot heap's freeze and the translation cache's stubs: under the rdb
# boot with a cache, a base closure's byte code (a boot object, frozen by the
# first collection) holds in its fourth word a stub, a small raw vector the
# freeze leaves ordinary (NO_FREEZE) and the frozen byte code reaches. The
# freeze meets the stub only through that frozen parent, and logs the
# parent. Each closure below is first called after the freezing collection,
# so its call replaces the stub with the translation (a store into frozen
# byte code), and the collections between the calls must keep the stub, then
# the translation, alive through that parent.
invisible(gc())
f <- function(k) {
  r <- switch(k,
    rev(c(3L, 1L, 2L)),
    trimws("  kept  "),
    tabulate(c(1L, 3L, 3L, 5L)),
    vapply(1:3, function(i) i * 2, numeric(1)),
    mapply(function(a, b) a + b, 1:3, 4:6),
    Reduce(`+`, 1:4, accumulate = TRUE),
    Filter(function(x) x > 1, 1:4),
    nlevels(factor(c("x", "y", "x"))),
    startsWith(c("apple", "banana"), "a"),
    sprintf("%05.1f", pi))
  invisible(gc())
  r
}
for (k in 1:10) print(f(k))
invisible(gc())
for (k in 1:10) print(f(k))
