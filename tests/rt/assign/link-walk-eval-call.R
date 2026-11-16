# The link-walk family (link-walk.R): eval of a call that rewrites itself.
# `.Internal(eval(ex, ...))` keeps the call in a C local, and its argument
# holds no link while it runs (evalList, BUILTIN_CALL_FRAME_ARGS): ex,
# built by as.call so that only its binding references it, is written in
# place, and `{`, which walks its call as it goes, evaluates the new third
# element: "new". eval's closure keeps expr in a promise, so the write
# copies: "old"; and so does a second binding.
mk <- function() {
  ex <<- as.call(list(as.name("{"), quote(ex[[3]] <<- "new"), "old"))
  NULL
}
g <- function(...) .Internal(eval(ex, environment(), NULL))
mk()
print(g())
print(ex[[3]])
mk()
print(g())
mk()
print(g())
h <- function() eval(ex, environment())
mk()
print(h())
print(ex[[3]])
mk()
ex2 <- ex
print(g())
print(ex2[[3]])
