# After the JIT compiles a closure (its third call), GNU R's compiled
# switch() ignores the tag on its first argument, so switch(a = 1L, EXPR =
# "a") stops being an error (issue #280). robigo always errors.
f <- function() switch(a = 1L, EXPR = "a")
for (i in 1:3) print(tryCatch(f(), error = conditionMessage))
