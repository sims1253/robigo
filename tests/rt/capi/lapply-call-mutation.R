# Package C writes into the X[[i]] of lapply's FUN(X[[i]], ...), which it
# reaches through sys.call(): the next invocation's call is GNU R's fresh
# one (robigo-rt keeps one call per isolate and checks it before reuse).
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/call_mutation.so"))
for (r in c("rt_call_set_index", "rt_call_set_tag", "rt_call_set_attr")) {
  invisible(lapply(1, function(v) {
    .Call(r, sys.call())
    NULL
  }))
  print(unlist(lapply(11:13, identity)))
  print(lapply(1, function(v) substitute(v)))
  print(attributes(lapply(1, function(v) substitute(v))[[1]]))
}
# vapply, in a promise of another closure's call: that closure's `[[`
invisible(vapply(1:2, identity, 0L))
probe <- function(X, FUN, FUN.VALUE, USE.NAMES = TRUE, ...) {
  `[[` <- function(x, i) -i
  vapply(1L, identity, {
    z <- .Internal(vapply(X, FUN, FUN.VALUE, USE.NAMES))
    print(z)
    0L
  })
}
invisible(probe(11:13, identity, 0L))
probe2 <- function(X, FUN, ...) {
  `[[` <- function(x, i) -i
  lapply(1L, function(v, ...) list(...), {
    z <- .Internal(lapply(X, FUN))
    print(unlist(z))
    0L
  })
}
invisible(probe2(11:13, identity))
