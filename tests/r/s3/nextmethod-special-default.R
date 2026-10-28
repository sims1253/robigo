# NextMethod() into an internal default of a special primitive (rep):
# the `rep.default` call must not dispatch again (eval.c DispatchOrEval)
rep.zz <- function(x, ...) structure(NextMethod(), class = "zz")
z <- structure(1:2, class = "zz")
unclass(rep(z, length.out = 3))
unclass(rep_len(z, 5))
x <- as.POSIXct("2009-08-07 00:00:01", tz = "UTC")
rep(x, 2)
rep_len(x, 3)
