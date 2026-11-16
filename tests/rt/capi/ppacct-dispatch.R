# check_stack_balance through S3 dispatch: UseMethod's own protections and
# its longjmp back to the generic (which restores the protect stack),
# NextMethod's .Internal, an internal generic's and a group method's.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/ppleak.so"))
mark <- function(s) cat("##", s, "\n", file = stderr())
mark("print"); print(.Call("leak"))
g <- function(x) UseMethod("g"); g.default <- function(x) .Call("leak")
mark("usemethod"); g(1)
g2 <- function(x) UseMethod("g2"); g2.a <- function(x) NextMethod(); g2.default <- function(x) .Call("leak")
mark("nextmethod"); g2(structure(1, class = "a"))
length.foo <- function(x) { .Call("leak"); 1L }
mark("internal-generic"); length(structure(1, class = "foo"))
"+.foo" <- function(e1, e2) .Call("leak")
mark("ops"); structure(1, class = "foo") + 1
mark("end")
