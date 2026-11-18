# matchArgs_NR keeps a used flag per actual (ARGUSED, match.c:185-383): a
# tagged empty argument (`expr=`) stays used after the positional pass fills
# its formal again (match.c:317, CAR(a) is still R_MissingArg), so only the
# actuals never used are unused, or go to `...`.
err <- function(expr) tryCatch(expr, error = function(e) conditionMessage(e))
# 1. exact tag
f <- function(expr, env) "ran"
err(f(expr=, a, env=list(), extra))
err(f(expr=, a, env=list(), extra, more = 2))
# 2. partial tag
err(f(ex=, a, env=1, extra))
# 3. `...` among the formals: what the dots receive
g <- function(expr, ...) c(n = ...length(), names = paste(...names(), collapse = ","))
err(g(expr=, a, b))
err(g(ex=, a, z = 2))
# 4. an empty argument in the middle, then too many
h <- function(x, y) c(missing(x), missing(y))
err(h(x=, , 3))
err(h(x=, , 3, 4))
# 5. a plain unused argument, a tagged one, from a promise's expression
k <- function(x) x
err(k(1, 2))
err(k(1, y = 2 + 3))
w <- function(...) k(...)
err(w(1, sin(2)))
# 6. a special's own formals (on.exit: expr, add, after)
u <- function() { on.exit(1, TRUE, FALSE, 4); "ran" }
err(u())
v <- function() { on.exit(1, add = TRUE, foo = 2, 3, 4); "ran" }
err(v())
err(substitute(a, list(), 3))
