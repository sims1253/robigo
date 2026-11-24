# protect-overflow.R's overflows under gctorture: the condition made and
# signalled while every allocation collects, caught twice, and the
# session going on.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/protect_overflow.so"))
over <- function() .Call("rt_protect_n", 60000L)
gctorture(TRUE)
r1 <- tryCatch(over(), protectStackOverflowError = function(e) conditionMessage(e))
r2 <- tryCatch(
  withCallingHandlers(over(), stackOverflowError = function(e) cat("calling handler\n")),
  error = function(e) class(e))
gctorture(FALSE)
print(r1)
print(r2)
.Call("rt_protect_n", 1000L)
cat("still going\n")
