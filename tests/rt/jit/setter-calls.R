# The replacement call a complex assignment reports (the old runtime's
# vm_setter_jit_oracle fixture, re-recorded on 4.6.2): the AST's call holds
# the right-hand side's value, compiled code's its expression.
f <- function(b, i, v) { b[[i]] <- v; b }
for (attempt in 1:2) {
    tryCatch(f(c(1, 2, b = c(x = 3)), c("b", "x"), 10), error = function(e) {
        print(conditionCall(e))
        cat(conditionMessage(e), "\n")
    })
}
