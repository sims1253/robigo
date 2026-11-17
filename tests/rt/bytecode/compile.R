# compiler::cmpfun's other cases and compiler::compile: a primitive is
# returned as it is, another value is an error; the function's attributes
# are kept; compile() makes a code object that eval() runs. The compiled
# closures run their source until V3 translates byte code.
err <- function(expr) tryCatch(expr, error = function(e) conditionMessage(e))
print(identical(compiler::cmpfun(sum), sum))
print(err(compiler::cmpfun(1)))
local({
  k <- compiler::cmpfun(function(n) if (n <= 1) 1 else n * k(n - 1))
  print(k(10))
  make <- compiler::cmpfun(function(a) function(b) a * b)
  print(make(3)(4))
  s <- compiler::cmpfun(function(v) { t <- 0; for (e in v) t <- t + e; t })
  print(s(1:100))
  m <- compiler::cmpfun(function(...) list(...))
  print(m(1, b = "two"))
  h <- structure(function() "h", class = c("myfun", "function"), note = "kept")
  hc <- compiler::cmpfun(h)
  print(class(hc))
  print(attr(hc, "note"))
  print(hc())
  invisible(NULL)
})
code <- compiler::compile(quote(1 + 2 * 3))
print(typeof(code))
print(eval(code))
x <- 5
print(eval(compiler::compile(quote(x * 2))))
compiler::disassemble(compiler::compile(quote(x * 2)))
print(err(compiler::disassemble(function(x) x)))
print(err(compiler::disassemble(1)))
