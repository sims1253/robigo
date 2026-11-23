# INTERIM(C1) (recorded, not ratcheted; order, owned by PR B, #1029): a
# literal's funEnv warnings come before a later force that fails (GNU R
# gives each warning when it occurs; the compile's error, a failed force,
# is tryCmpfun's, silent). robigo-rt forces with the warnings pending and
# its error path drops them. LC_ALL=C: the escaped name does not translate.
invisible(compiler::enableJIT(2))
delayedAssign("myfun", { cat("force\n"); stop("boom") })
f <- function(x) {
    if (x) identity(function() { "\u4e2d" <- 1 })
    if (x) myfun()
    7
}
print(f(FALSE))
withCallingHandlers(print(f(FALSE)), warning = function(w) {
    cat("warning\n")
    invokeRestart("muffleWarning")
})
