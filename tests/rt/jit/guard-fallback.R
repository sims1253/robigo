# A failed BASEGUARD evaluates the call as the AST does (eval.c's
# DO_BASEGUARD: eval(expr, rho)): its constant argument is a promise, not
# compiled code's value.
gt <- function() typeof(1L)
print(gt()); print(gt())
typeof <- function(x) .Internal(getBindingType(quote(x), environment()))
print(gt())
print(gt())
