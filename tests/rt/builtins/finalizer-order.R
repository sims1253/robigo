# The order finalizers run in. GNU R prepends each registration to
# R_weak_refs (memory.c:NewWeakRef) and runs the ready ones walking that
# list from its head (RunFinalizers, R_RunExitFinalizers): newest first,
# after a collection and at exit.
reg <- function(name, onexit = FALSE) {
    e <- new.env()
    reg.finalizer(e, function(e) cat("finalizer", name, "\n"), onexit)
    e
}
collect <- function() invisible(.Internal(gc(FALSE, FALSE, TRUE)))
a <- reg("a"); b <- reg("b"); c <- reg("c")
a <- b <- c <- NULL
collect()
cat("after the first collection\n")
# several finalizers of one key
e <- new.env()
for (n in c("x", "y", "z")) local({
    n <- n
    reg.finalizer(e, function(e) cat("same key", n, "\n"))
})
e <- NULL
collect()
cat("after the second collection\n")
# a live key's finalizer waits; one registered later and dropped runs
kept <- reg("kept")
d <- reg("d")
d <- NULL
collect()
cat("after the third collection\n")
# at exit, the onexit ones, newest first ("kept" does not run)
p <- reg("exit p", TRUE)
q <- reg("exit q", TRUE)
cat("end of the script\n")
