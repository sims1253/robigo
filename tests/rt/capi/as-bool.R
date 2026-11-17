# Rf_asBool from package C (coerce.c:asBool): the first element as a
# logical; more than one element of an atomic vector is an error reported
# with no call, NA (also of a zero-length vector or a list) is error()'s,
# reported against the .Call.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/as_bool.so"))
.Call("rt_as_bool", TRUE)
.Call("rt_as_bool", 0L)
.Call("rt_as_bool", 2.5)
.Call("rt_as_bool", "T")
.Call("rt_as_bool", as.raw(0))
err <- function(x) {
  e <- tryCatch(.Call("rt_as_bool", x), error = function(e) e)
  print(conditionMessage(e))
  print(conditionCall(e))
}
err(c(TRUE, FALSE))
err(NA)
err(logical(0))
err(list(1))
err("maybe")
