# GNU R 4.5.0-3, automatic JIT (not eager robigo VM compilation).
invisible(compiler::enableJIT(3))
f <- function(b, i, v) { b[[i]] <- v; b }
for (attempt in 1:2) {
    tryCatch(f(c(1, 2, b = c(x = 3)), c("b", "x"), 10), error = function(e) {
        cat(paste(deparse(conditionCall(e)), collapse = " "), conditionMessage(e), sep = "\t")
        cat("\n")
    })
}
