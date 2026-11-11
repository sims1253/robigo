# Once the JIT compiles a closure (its second call here), GNU R's
# STARTSUBSET keeps the tag of `[`'s first argument (eval.c
# DO_STARTDISPATCH: SETCALLARG_TAG), so `drop = x` is taken as the drop
# argument and the subset has too few indices. The AST interpreter's
# R_DispatchOrEvalSP drops it (CONS_NR). A gap: robigo does not model the
# compiled tier's tag handling (compare lang/switch-compiled-tag, #280).
m <- matrix(1:6, nrow = 2, dimnames = list(c("a", "b"), LETTERS[1:3]))
h <- function(x) `[`(drop = x, 2, , drop = FALSE)
for (i in 1:3) print(tryCatch(h(m), error = function(e) list(conditionCall(e), conditionMessage(e))))
