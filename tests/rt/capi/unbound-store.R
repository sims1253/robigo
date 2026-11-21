# A native routine can return R_UnboundValue. A compiled local assignment of
# it into a new binding reaches defineVar's error (envir.c:defineVar), in the
# closure's first call and later (byte-code) ones (X9-2's direct slot store
# leaves it to defineVar).
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/identity.so"))
f <- function() { x <- .Call("rt_unbound"); "stored" }
print(tryCatch(f(), error = conditionMessage))
print(tryCatch(f(), error = conditionMessage))
print(tryCatch(f(), error = conditionMessage))
# into a binding the closure already has
g <- function() { x <- 1; x <- .Call("rt_unbound"); "stored" }
print(tryCatch(g(), error = conditionMessage))
print(tryCatch(g(), error = conditionMessage))
