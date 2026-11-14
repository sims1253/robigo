# The capi corpus's extptr case (tests/corpora/capi/cases/extptr.R) for
# robigo-rt, its C in support/extptr.c built by scripts/rt-capi-build.sh
# into RT_CAPI_LIBS: capi_extptr's protections balanced (the corpus case's
# UNPROTECT(7) after six PROTECTs relies on GNU R's evaluator's own
# protections below the .Call; ledger 2026-11-14, A1).
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/extptr.so"))

print(.Call("capi_extptr"))

p <- .Call("capi_make_finalized")
print(identical(p, p))
rm(p)
invisible(gc())  # runs the finalizer

key <- new.env()
val <- "value"
w <- .Call("capi_weakref", key, val)
print(w)
