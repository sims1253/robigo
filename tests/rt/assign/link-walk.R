# The links that keep a pending value from being written in place during a
# complex assignment are those of GNU R's node stack (INCLNK_stack_commit,
# eval.c:997-1016): a builtin's C locals hold none, and a builtin's own
# arguments hold none while it runs (evalList and BUILTIN_CALL_FRAME_ARGS
# drop their links before the call). vapply keeps X in a local while FUN
# runs and names a character X's result by X itself, so FUN's
# `x[2] <<- "Z"` on an unshared x writes in place and shows in the names.
# The same before and after GNU R's JIT compiles the caller (at its second
# call, or at its first when it has a loop).
# ROBIGO_NO_INPLACE=1 copies at every write, so this recording holds in
# production only (the .requires: inplace-identity); link-walk-*.R are the
# family, one shape each.
reset <- function() {
  x <<- c("a", "b", "c")
  NULL
}
g <- function(...) .Internal(vapply(x, function(e) { x[2] <<- "Z"; 1 }, 0, TRUE))
reset()
print(g())
print(x)
reset()
print(g())
reset()
print(g())
h <- function(...) {
  for (i in 1) NULL
  .Internal(vapply(x, function(e) { x[2] <<- "Z"; 1 }, 0, TRUE))
}
reset()
print(h())
# shared before the call: copied, the names keep "b"
reset()
x2 <- x
print(g())
print(x2)
# controls that already matched: a value pending in the caller is shared
# while the assignment runs; `names<-` copies the names vector; on.exit's
# code runs with the return value linked (endcontext)
y <- c(p = 1, q = 2)
k <- function(...) .Internal(vapply(y, function(e) { names(y)[1] <<- "zz"; e }, 0, TRUE))
print(k())
print(y)
x <- c(1, 2)
print(list(x, { x[1] <- 5; x })[[1]])
f <- function() { x <- c(1, 2); on.exit(x[1] <- 99); x }
print(f())
