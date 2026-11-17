# serialize() of closures over a closure frame (an unhashed environment
# whose bindings robigo-rt keeps in slots): GNU R's bytes in each format and
# version. (Apart from tests/rt/serialize, whose cases scripts/base-subset.sh
# traces: its tracer rewrites the closures it sees.)
hex <- function(x) paste(as.character(x), collapse = "")
show <- function(name, v) {
  for (ver in c(2L, 3L)) {
    x <- serialize(v, NULL, version = ver)
    cat(name, " v", ver, " xdr ", hex(x), "\n", sep = "")
    cat(name, " v", ver, " bin ", hex(serialize(v, NULL, xdr = FALSE, version = ver)), "\n", sep = "")
    cat(name, " v", ver, " asc ", hex(serialize(v, NULL, ascii = TRUE, version = ver)), "\n", sep = "")
    cat(name, " v", ver, " hex ", hex(serialize(v, NULL, ascii = NA, version = ver)), "\n", sep = "")
    cat(name, " v", ver, " back ", identical(unserialize(x), v), "\n", sep = "")
  }
}
fr <- (function() {
  y <- 1
  z <- "two"
  environment()
})()
g <- eval(quote(function(w) y + w), fr)
show("local-closure", g)
show("env-list", list(f = g, e = environment(g)))
