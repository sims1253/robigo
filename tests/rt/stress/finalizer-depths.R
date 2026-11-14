# Experiment A's stress script (docs/g3.md §15.1; docs/ledger/core.md,
# 2026-11-11): an external pointer with a C finalizer made by a .Call at R
# call depth a, then a collection at depth b, made by C's allocations or by
# gc(), for a and b in 0..24; the finalizer must have run once by the next
# point where GNU R runs pending finalizers. A stale word that kept the
# dead pointer alive (a missed finalizer) prints its (kind, a, b).
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/stress.so"))
deep <- function(n, f) if (n > 0L) deep(n - 1L, f) else f()
for (kind in c("churn", "gc")) {
    missed <- 0L
    for (a in 0:24) for (b in 0:24) {
        id <- (kind == "gc") * 625L + a * 25L + b
        deep(a, function() invisible(.Call("pointer", id)))
        if (kind == "churn") deep(b, function() invisible(.Call("churn", 200L)))
        else deep(b, function() invisible(gc()))
        for (i in 1:2000) NULL  # (eval's poll runs pending finalizers)
        n <- .Call("finalized", id)
        if (n != 1L) {
            missed <- missed + 1L
            cat(kind, a, b, "finalized", n, "times\n")
        }
    }
    cat(kind, ": missed", missed, "of 625\n")
}
