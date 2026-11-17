# The bytecode-fidelity fixtures' functions (tests/r/serialize/bytecode-
# fixtures/generate.r, generate-constructed.r, compiled-fixtures/source.txt
# and the live comparisons in bytecode_construct.R and
# bytecode_compiled_files.R), compiled by cmp.R from source on robigo-rt:
# disassemble() prints GNU R's code and constants for each. (The fixtures
# themselves are serialized; reading them is L1's.)
local({
  # cmpfun-simple.rds, and the live cf_live
  compiler::disassemble(compiler::cmpfun(function(x) x + 1L))
  # cf_math
  compiler::disassemble(compiler::cmpfun(function(x, y = 2L) { z <- 10L; (x + y) * z + 1L }))
  # generate.r's closure, without source references, and cf_nested
  compiler::disassemble(compiler::cmpfun(function(x = 3L) {
    # Preserve source and nested bytecode constants.
    g <- function(y) y + 1L
    for (i in seq_len(x)) x <- g(x)
    x
  }))
  # the promise fixtures' g
  f <- function(x) environment()
  g <- compiler::cmpfun(function() { z <- 40L; f(z + 2L) })
  compiler::disassemble(g)
  print(g()$x)
  # f_sq
  f_sq <- compiler::cmpfun(function(a, b = 3L) a * a + b)
  compiler::disassemble(f_sq)
  print(f_sq(4L))
  print(f_sq(4L, 10L))
  invisible(NULL)
})
# source.txt's expression, as cmpfile compiles it
compiler::disassemble(compiler::compile(quote(loaded_compiled <- function(x=1L) x+42L)))
