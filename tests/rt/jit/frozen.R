# Compiled code keeps base's language functions as they were when it was
# compiled (cmp.R inlines them without a guard); code compiled later, the
# AST and a compiled top-level loop find the binding in force.
f <- function(x) x + 1
print(f(1)); print(f(1))
`+` <- function(e1, e2) "user plus"
print(f(1))
g <- function(x) x + 1
print(g(1))
print(g(1))
print(1 + 1)
for (i in 1) print(2 + 2)
`if` <- function(cond, yes, no) "user if"
h <- function() if (TRUE) "base if"
print(f(2)); print(h()); print(h())
