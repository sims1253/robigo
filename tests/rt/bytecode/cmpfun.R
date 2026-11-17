# cmp.R's cmpfun, run from source on robigo-rt (the compiler package's
# namespace is made from the pinned tarball's cmp.R), gives GNU R's byte
# code: disassemble() prints the opcodes and constants as cmp.R decodes
# them (bcDecode), through dput. The compiled closure keeps its formals,
# body() and environment. Closures stay inside local(): the base-subset
# tracer rewrites closures bound in the global environment.
local({
  f <- function(x, y = 2) x + y
  g <- compiler::cmpfun(f)
  print(typeof(g))
  print(g(1))
  print(g(1, 10))
  print(body(g))
  print(formals(g))
  print(identical(environment(g), environment(f)))
  print(identical(f, g))
  print(identical(f, g, ignore.bytecode = FALSE))
  print(identical(g, g, ignore.bytecode = FALSE))
  print(deparse(g))
  compiler::disassemble(g)
  invisible(NULL)
})
local(compiler::disassemble(compiler::cmpfun(function(x) x + 1L)))
local(compiler::disassemble(compiler::cmpfun(function(n) if (n <= 1) 1 else n * k(n - 1))))
local(compiler::disassemble(compiler::cmpfun(function(v) { t <- 0; for (e in v) t <- t + e; t })))
local(compiler::disassemble(compiler::cmpfun(function(...) list(...))))
local(compiler::disassemble(compiler::cmpfun(function(a) function(b) a * b)))
local(compiler::disassemble(compiler::cmpfun(function(x, y = 2L) { z <- 10L; (x + y) * z + 1L })))
local(compiler::disassemble(compiler::cmpfun(function(x = 3L) {
  g <- function(y) y + 1L
  for (i in seq_len(x)) x <- g(x)
  x
})))
invisible(NULL)
