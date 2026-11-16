# A call rewritten from C while it runs (docs/g3.md §9.3): GNU R made the
# call's argument list (promargs) when the call began, so UseMethod's
# method sees the arguments' names and missingness as they were then. The
# callers never compile in GNU R (they may call browser()); robigo runs them
# as code whose call sites keep match plans, used from the second call on.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/closure_mutation.so"))
mutate <- FALSE
# the call's names removed: the method is still matched by them
f <- function(a, b) {
  if (mutate) invisible(.Call("rt_unname_call", h, 2L))
  UseMethod("f")
}
f.numeric <- function(b, a) print(list(a = a, b = b))
h <- function() { if (FALSE) browser(); f(a = 1, b = 2) }
h(); h()
mutate <- TRUE
h()
mutate <- FALSE
h(); h()
# the call's empty argument filled: the method still sees it missing
g <- function(a, b) {
  if (mutate) invisible(.Call("rt_set_path", k, c(2L, 2L), 5))
  UseMethod("g")
}
g.numeric <- function(a, b) cat(" missing(b)", missing(b), "\n")
k <- function() { if (FALSE) browser(); g(1, ) }
k(); k()
mutate <- TRUE
k()
mutate <- FALSE
k(); k()
