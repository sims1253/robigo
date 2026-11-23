# PROTECT past R_PPStackSize (memory.c:R_signal_protect_error): the
# condition is a protectStackOverflowError and a stackOverflowError, with
# GNU R's message and a NULL call; its calling handlers run (the red zone
# opened for them), and once the error has left, the stack's size is back:
# a second overflow is caught again, with its handlers run again, and the
# session goes on. An exit out of a calling handler restores the size
# before any cleanup around the overflow runs.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/protect_overflow.so"))
over <- function() .Call("rt_protect_n", 60000L)
.Call("rt_protect_n", 1000L)
# (the helper refuses a count that is not one)
for (n in list(NA_integer_, -1L)) print(tryCatch(.Call("rt_protect_n", n), error = function(e) conditionMessage(e)))
show <- function(e) {
  print(class(e))
  print(conditionMessage(e))
  print(conditionCall(e))
}
show(tryCatch(over(), protectStackOverflowError = function(e) e))
show(tryCatch(over(), stackOverflowError = function(e) e))
for (i in 1:2) {
  r <- tryCatch(
    withCallingHandlers(over(), protectStackOverflowError = function(e) cat("calling handler", i, "\n")),
    error = function(e) conditionMessage(e))
  print(r)
}
# a calling handler may protect in the red zone
r <- tryCatch(
  withCallingHandlers(over(), error = function(e) cat("in the red zone:", .Call("rt_protect_n", 500L), "\n")),
  error = function(e) "caught")
print(r)
# an exit out of a calling handler (an error, an outer restart) passes the
# overflow's context first, which restores the size, before the cleanups
# around it
.Call("rt_pp_size")
r <- tryCatch(.Call("rt_size_in_cleanup", quote(withCallingHandlers(over(),
  protectStackOverflowError = function(e) stop("from the handler"))), environment()),
  error = function(e) conditionMessage(e))
print(r)
withRestarts(.Call("rt_size_in_cleanup", quote(withCallingHandlers(over(),
  protectStackOverflowError = function(e) invokeRestart("outer"))), environment()),
  outer = function() cat("outer restart\n"))
.Call("rt_pp_size")
.Call("rt_protect_n", 1000L)
cat("still going\n")
