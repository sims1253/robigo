# An error raised in a chain of calls robigo may run inline (pure closures
# called from compiled code, each reading its argument first): the first
# iterations run inline; the one that errs gives up and runs for real, so
# Rscript's traceback lists real frames and the error's call is the inner
# one.
inner <- function(x) if (x > 3) x + "b" else x
mid <- function(x) if (x > 0) inner(x) * 2 else 0
outer <- function(x) if (x > 0) mid(x) - 1 else 0
run <- function() {
  s <- 0
  for (i in 1:5) s <- s + outer(i)
  s
}
print(outer(1))
run()
