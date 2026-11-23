# Restore the assignment call before an error reaches a condition handler.
f <- function(m) {
    for (i in 1:41) m[[i, 1]] <- i
}
tryCatch(f(matrix(0, 40, 1)), error = function(e) {
    print(conditionCall(e))
    print(conditionMessage(e))
    print(class(e))
})
