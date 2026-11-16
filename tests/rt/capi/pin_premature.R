# The capi corpus's pin_premature case (tests/corpora/capi/cases/pin_premature.R) for
# robigo-rt, its C in support/pin_premature.c built by scripts/rt-capi-build.sh
# into RT_CAPI_LIBS.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/pin_premature.so"))

# An unprotected external pointer passed to an allocating API function is
# not finalized during the call (docs/g3.md §3 item 6, §15.1): TRUE would
# be a finalizer that ran while the pointer was an argument. The second
# binds it in an environment made in R, the process's first write from C
# into one (processx__create_connection).
gctorture(TRUE)
early <- .Call("capi_pp_probe", new.env())
gctorture(FALSE)
print(early)
