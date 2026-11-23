# tryCmpfun (#964): the JIT is off while a compile runs, and an exit that
# leaves it leaves the level as it is then: a warning handler's
# enableJIT() stands, as GNU R restores R_jit_enabled only when the
# compile returns. Run under LC_ALL=C.
f <- function() { "\u4e2d" <- 1; for (i in 1) NULL; 0 }
r <- try(withCallingHandlers(f(), warning = function(w) {
    cat("level in the handler:", compiler::enableJIT(2), "\n")
    stop("leave")
}), silent = TRUE)
cat(r)
cat("JIT:", compiler::enableJIT(3), "\n")
g <- function() { "\u4e2d" <- 2; for (i in 1) NULL; 0 }
r <- try(withCallingHandlers(g(), warning = function(w) stop("leave")), silent = TRUE)
cat(r)
cat("JIT:", compiler::enableJIT(3), "\n")
