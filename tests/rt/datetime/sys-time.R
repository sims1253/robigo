# times.c do_systime (base's Sys.time() wraps it in .POSIXct): the wall
# clock in seconds since the epoch, with sub-second digits. Nothing printed
# here depends on the day the case runs.
t1 <- Sys.time()
t2 <- Sys.time()
class(t1)
typeof(unclass(t1))
length(t1)
is.null(attr(t1, "tzone"))
as.numeric(t2) >= as.numeric(t1)
x <- .Internal(Sys.time())
typeof(x); length(x); is.null(attributes(x))
x > 1.7e9 && x < 1e11
x != floor(x) || .Internal(Sys.time()) != floor(.Internal(Sys.time()))
r <- tryCatch(.Internal(Sys.time(1)), error = function(e) conditionMessage(e)); r
