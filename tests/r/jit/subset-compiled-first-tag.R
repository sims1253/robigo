# The tag of `[`'s (and `[[`'s) first argument. The AST interpreter's
# R_DispatchOrEvalSP conses the object on untagged, for objects too, so
# `drop = x` is just the object; a leading `...` skips that path, and
# DispatchOrEval evaluates the dots with their tags. Once GNU R compiles
# the code (a closure from its second call, a top-level loop at once),
# STARTSUBSET keeps the tag (eval.c DO_STARTDISPATCH: SETCALLARG_TAG):
# `drop = x` is taken as the drop argument and the subset has too few
# indices. Calls with `...`, or of a local `[`, are not inlined.
m <- matrix(1:6, nrow = 2, dimnames = list(c("a", "b"), LETTERS[1:3]))
h <- function(x) `[`(drop = x, 2, , drop = FALSE)
for (i in 1:3) print(tryCatch(h(m), error = function(e) list(conditionCall(e), conditionMessage(e))))
p <- function(e) print(tryCatch(e, error = function(e) conditionMessage(e)))
# a top-level loop is compiled, with the promises it makes
for (i in 1:2) p(`[`(drop = m, 2, , drop = FALSE))
h2 <- function(x) `[[`(exact = x, "a", exact = TRUE)
for (i in 1:3) p(h2(list(a = 1)))
h3 <- function(x) `[[`(exact = x, "a")
for (i in 1:3) p(h3(list(a = 1)))
# `...` in the call: not inlined
h4 <- function(x, ...) `[`(drop = x, ...)
for (i in 1:3) p(h4(m, 2, , drop = FALSE))
h5 <- function(x) `[`(x = x, 2, , drop = FALSE)
for (i in 1:3) p(h5(m))
# an object: untagged to the method in the AST interpreter, tagged once compiled
df <- data.frame(a = 1:2, b = 3:4)
h7 <- function(x) `[`(drop = x, 2, )
for (i in 1:3) p(suppressWarnings(h7(df)))
# a local `[` is not inlined
h8 <- function(x) { `[` <- base::`[`; `[`(drop = x, 2, , drop = FALSE) }
for (i in 1:3) p(h8(m))
# a promise compiled code made is compiled
h9 <- function(x) identity(`[`(drop = x, 2, , drop = FALSE))
for (i in 1:3) p(h9(m))
h12 <- function(x) `[[`(drop = x, 1)
for (i in 1:3) p(h12(list(1)))
# a leading `...`: DispatchOrEval keeps the dots' tags, compiled or not
f <- function(...) `[`(...)
for (i in 1:3) p(f(drop = m, 2, , drop = FALSE))
g <- function(...) `[[`(...)
for (i in 1:3) p(g(exact = list(a = 1), "a", exact = TRUE))
for (i in 1:3) p(suppressWarnings(f(drop = df, 2, )))
