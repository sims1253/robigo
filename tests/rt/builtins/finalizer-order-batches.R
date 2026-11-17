# The order of finalizers readied by different collections. GNU R keeps a
# ready reference in R_weak_refs (newest first) until it runs, and one
# RunFinalizers pass walks that list once from its head, not reentered
# while a finalizer runs (memory.c). So, of two references readied by two
# collections inside one finalizer, the newer runs first; and a reference
# newer than the running finalizer, readied during it, waits for the next
# pass (the walk only goes on to older ones).
reg <- function(name) {
    e <- new.env()
    reg.finalizer(e, function(e) cat("finalizer", name, "\n"))
    e
}
collect <- function() invisible(.Internal(gc(FALSE, FALSE, TRUE)))
x1 <- reg("1"); x2 <- reg("2"); x3 <- reg("3")
outer <- new.env()
reg.finalizer(outer, function(e) {
    cat("outer starts\n")
    # two collections, each readying one reference: the older first
    x1 <<- NULL
    collect()
    x3 <<- NULL
    collect()
    # and one registered after this one
    x6 <<- NULL
    collect()
    cat("outer ends\n")
})
x6 <- reg("6")
outer <- NULL
collect()
cat("after the first collection\n")
collect()
cat("after the second collection\n")
x2 <- NULL
collect()
cat("end\n")
