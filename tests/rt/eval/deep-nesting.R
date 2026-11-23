# An expression nested deeper than the compiler goes (#976: robigo-rt
# compiled 20,000 levels on the native stack and crashed) is evaluated as
# GNU R evaluates it: below options(expressions) its value, past it GNU R's
# error, both evaluated directly and as a closure's body (which the JIT
# would compile).
d <- 1
for (i in 1:900) d <- call("+", 1, d)
print(eval(d))
d <- 1
for (i in 1:20000) d <- call("+", 1, d)
print(tryCatch(eval(d), error = function(e) conditionMessage(e)))
f <- function() NULL
body(f) <- d
print(tryCatch(f(), error = function(e) conditionMessage(e)))
print(tryCatch(f(), error = function(e) conditionMessage(e)))
