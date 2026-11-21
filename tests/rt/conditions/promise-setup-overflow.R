# R: eval.c:setup_bcframe_prom. Byte code's GETVAR forces an unforced
# promise of byte code in place: PRSEEN 1 and the depth step come before
# the promise is pending, so an overflow at that step leaves it PRSEEN 1
# ("promise already under evaluation" later), while the promises above it
# are marked interrupted (the restart warning later).
k <- compiler::cmpfun(function(n, d) if (d == 0L) environment() else k(n + 1L, d - 1L))
smallest <- function(D) {
    for (L in 25:200) {
        e <- k(1L, D)
        options(expressions = L)
        r <- tryCatch(e$n, error = function(x) "E")
        options(expressions = 5000L)
        if (!identical(r, "E")) return(L)
    }
    NA
}
for (D in c(20L, 40L)) {
    L <- smallest(D)
    e <- k(1L, D)
    options(expressions = L - 3L)
    r1 <- tryCatch(e$n, error = function(x) conditionMessage(x))
    options(expressions = 5000L)
    W <- 0L
    r2 <- tryCatch(withCallingHandlers(e$n, warning = function(w) {
        W <<- W + 1L
        invokeRestart("muffleWarning")
    }), error = function(x) conditionMessage(x))
    cat("D =", D, "\n")
    print(r1)
    print(r2)
    cat("restart warnings:", W, "\n")
}
