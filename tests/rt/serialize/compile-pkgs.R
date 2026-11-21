# serialize() under compilePKGS(TRUE) (R_compile_pkgs, serialize.c's
# WriteItem): each closure whose body is not byte code is compiled as it is
# written (eval.c's R_cmpfun1: compiler:::tryCmpfun), and the bytes are
# those of the compiled closure. R_compile_pkgs is off while the compiled
# closure is written, so a closure in its environment is written as it is,
# and TRUE after. A closure already compiled, and one the compiler gives
# back uncompiled, are written as they are. Closures stay inside local():
# the base-subset tracer rewrites closures bound in the global environment.
hex <- function(x) paste(as.character(x), collapse = "")
show <- function(name, v) {
  for (ver in c(2L, 3L)) {
    x <- serialize(v, NULL, version = ver)
    cat(name, " v", ver, " ", length(x), " ", hex(x), "\n", sep = "")
  }
}
local({
  f <- function(x, y = 2) { z <- x + y; if (z > 3) z * 2 else -z }
  h <- local({ k <- function(a) a + 1; function(b) k(b) * 2 })
  m <- function(n) { s <- 0; for (i in seq_len(n)) s <- s + i; s }
  w <- function(...) list(...)
  print(compiler::compilePKGS(TRUE))
  show("plain", f)
  show("env", h)
  show("loop", m)
  show("dots", w)
  show("list", list(f, 1, m))
  show("compiled", compiler::cmpfun(f))
  # (a closure the compiler declines: a call of browser() in its body)
  show("browser", function() if (FALSE) browser())
  print(typeof(.Internal(bodyCode(unserialize(serialize(f, NULL))))))
  print(unserialize(serialize(h, NULL))(1))
  print(compiler::compilePKGS(FALSE))
  show("off", f)
})
