# INTERIM(C1) (recorded, not ratcheted; order, owned by PR B, #1029): a
# warning handler removes a shadow of `quote` in the middle of a compile.
# GNU R finishes the call it is in (all three warnings of the first
# literal, then the second's, each literal's reads after its funEnv's
# warnings); robigo-rt's retry compiles the changed code from the start, and
# what it counted as raised and read belongs to the code before the change.
# LC_ALL=C: the escaped names do not translate.
invisible(compiler::enableJIT(2))
o <- compiler:::compilerOptions
rm("optimize", envir = o)
makeActiveBinding("optimize", function() { cat("read\n"); 2 }, o)
quote <- function(...) NULL
f <- function(x) {
    if (x) quote(function() { "\u4e2d" <- 1 })
    if (x) identity(function() { "\u6587" <- 1 })
    7
}
print(f(FALSE))
withCallingHandlers(print(f(FALSE)), warning = function(w) {
    cat(if (grepl("4E2D", conditionMessage(w))) "first\n" else "second\n")
    if (exists("quote", .GlobalEnv, inherits = FALSE))
        rm("quote", envir = .GlobalEnv)
    invokeRestart("muffleWarning")
})
