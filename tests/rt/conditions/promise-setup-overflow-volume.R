# R: eval.c:setup_bcframe_prom. Byte code's GETVAR (and DDVAL) forces an
# unforced promise of byte code in place: PRSEEN 1 and the depth step come
# before the promise is pending, so an overflow at that step leaves it
# PRSEEN 1 ("promise already under evaluation" later), while the promises
# above it are marked interrupted (the restart warning later). Two chains:
# argument promises, and `...` promises read by a compiled ..1.
k <- compiler::cmpfun(function(n, d) if (d == 0L) environment() else k(n + 1L, d - 1L))
kd <- compiler::cmpfun(function(..., d) if (d == 0L) environment() else kd(..1 + 1L, d = d - 1L))
gd <- compiler::cmpfun(function() ..1)
probe <- function(label, mk, force) {
    mk
    force
    L <- 25L
    repeat {
        e <- mk()
        options(expressions = L)
        r <- tryCatch(force(e), error = function(x) "E")
        options(expressions = 5000L)
        if (!identical(r, "E") || L >= 200L) break
        L <- L + 1L
    }
    e <- mk()
    options(expressions = L - 3L)
    r1 <- tryCatch(force(e), error = function(x) conditionMessage(x))
    options(expressions = 5000L)
    W <- 0L
    r2 <- tryCatch(withCallingHandlers(force(e), warning = function(w) {
        W <<- W + 1L
        invokeRestart("muffleWarning")
    }), error = function(x) conditionMessage(x))
    cat(label, "\n")
    print(r1)
    print(r2)
    cat("restart warnings:", W, "\n")
}
probe("argument promises, read by $", function() k(1L, 20L), function(e) e$n)
probe("... promises, read by ..1", function() kd(1L, d = 20L), function(e) {
    environment(gd) <- e
    gd()
})
