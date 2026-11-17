# Rdynload.c: the table of loaded DLLs and base's registered routines
# (R_init_base, registration.c), as baseloader.R binds them.
d <- getLoadedDLLs()
class(d)
names(d)
b <- d[["base"]]
class(b)
names(b)
b[["name"]]
b[["path"]]
b[["dynamicLookup"]]
b[["forceSymbols"]]
class(b[["handle"]])
typeof(b[["handle"]])
class(b[["info"]])
typeof(b[["info"]])
r <- getDLLRegisteredRoutines(b)
class(r)
names(r)
vapply(r, length, 0L)
vapply(r, function(l) class(l), "")
names(r$.Call)
names(r$.Fortran)
s <- r$.Call$R_addTaskCallback
class(s)
names(s)
s$name
s$numParameters
class(s$address)
typeof(s$address)
class(s$dll)
s$dll[["name"]]
vapply(r$.Call, function(s) s$numParameters, 0L)
vapply(r$.Fortran, function(s) s$numParameters, 0L)
vapply(r$.Fortran, function(s) class(s)[1L], "")
# the same routines again: the same names, new references
getDLLRegisteredRoutines(b)$.Call$R_addTaskCallback$name
getDLLRegisteredRoutines.DLLInfo(b, addNames = FALSE)$.Fortran[[6L]]$name
is.null(names(getDLLRegisteredRoutines(b, FALSE)$.Call))
# references made and dropped: their copies are freed by their finalizers
for (i in 1:50) x <- getDLLRegisteredRoutines(b)
x <- NULL
invisible(gc())
getNativeSymbolInfo("dtrco", b, withRegistrationInfo = TRUE)$numParameters
# one DLLInfoReference per DLL, kept while it is loaded (DLLInfoEptrs)
i <- getLoadedDLLs()[["base"]][["info"]]
attr(i, "mark") <- "kept"
attr(getLoadedDLLs()[["base"]][["info"]], "mark")
attr(getDLLRegisteredRoutines(b)$.Fortran$dtrco$dll[["info"]], "mark")
reg.finalizer(i, function(e) cat("finalized while loaded\n"))
i <- NULL
invisible(gc())
"after gc"
