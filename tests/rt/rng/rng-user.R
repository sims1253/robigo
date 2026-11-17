# RNG_Init's R_FindSymbol("user_unif_rand"): no loaded object has one
show <- function(expr) tryCatch(expr, error = function(e) cat("Error:", conditionMessage(e), "\n"))
show(RNGkind("user"))
show(RNGkind(normal.kind = "user"))
RNGkind()
