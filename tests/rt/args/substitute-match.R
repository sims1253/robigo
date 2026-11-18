# substitute()'s own argument matching (coerce.c do_substitute,
# matchArgs_NR): its errors against the call.
show <- function(expr) {
  r <- tryCatch(expr, error = function(e) e)
  print(r)
  if (inherits(r, "error")) print(conditionCall(r))
  invisible()
}
substitute(a + b, list(a = 1))
substitute(env = list(a = 2), expr = a)
show(substitute(e = a, list(a = 3)))
show(substitute(a, b, c))
show(substitute(a, env = b, c, d))
show(substitute(a, foo = 1))
show(substitute(e = 1, ex = 2))
show(substitute(a, env = 1, env = 2))
show(substitute(a, 1))
f <- function(x) substitute(x, 1, 2)
show(f(y))
