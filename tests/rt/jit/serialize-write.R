# serialize() of closures in each of the JIT's states, without
# _R_COMPILE_PKGS_: one the JIT compiled has R_cmpfun1's byte code as its
# BODY (eval.c:R_cmpfun) and is written with it; one it has not (called
# once and marked MAYBEJIT, never called, or marked NOJIT) is written as
# its source, its JIT bits in the flags. Each closure has a body of its
# own (the JIT cache).
hex <- function(x) paste(as.character(x), collapse = "")
show <- function(name, f) {
  for (ver in c(2L, 3L)) {
    x <- serialize(f, NULL, version = ver)
    cat(name, " v", ver, " ", length(x), " ", hex(x), "\n", sep = "")
  }
}
# a loop: compiled at its first call
f_loop <- function(n) { s <- 0; for (i in seq_len(n)) s <- s + i; s }
f_loop(3)
show("loop", f_loop)
# a small top-level closure: MAYBEJIT at its first call, compiled at its second
f_small <- function(x) x * 2 + 1
f_small(1)
show("maybe", f_small)
f_small(2)
show("small", f_small)
# never called
f_none <- function(y) y - 1
show("none", f_none)
# a small closure of a local environment: NOJIT at its first call
local({
  g <- function(z) z + 10
  g(1)
  show("nojit", g)
})
# read back, the compiled one runs
h <- unserialize(serialize(f_loop, NULL))
print(h(4))
