# cmp.R's constant folding in compiled code (GNU R 4.6.2). A call of a
# base fold function on constants compiles as its value, which joins the
# identical() constant already in the code's pool (0 and -0 are
# identical, so a folded -0 then loads as 0). In global code only the
# language functions (`-`, `(`, `:`, ...) are base variables for cmp.R, so
# `rep()` and `c()` are calls there (their compiled arguments still fold);
# in a namespace every fold function folds, to a value at most 10 long. A
# test that folds drops the branch it does not take; a fold that errs or
# warns is left to run. A global closure with a loop is compiled at its
# first call, one in a namespace at its second.
zero <- function() { a <- 0; b <- -0; for (i in 1:2) v <- b; 1 / v }
print(zero())
alone <- function() { b <- -0; for (i in 1:2) v <- b; 1 / v }
print(alone())
nested <- function() { a <- 0; b <- -(1 - 1); for (i in 1:2) v <- b; 1 / v }
print(nested())
paren <- function() { a <- 0; b <- (-0); for (i in 1:2) v <- b; 1 / v }
print(paren())
dead <- function() { if (FALSE) a <- 0; b <- -0; for (i in 1:2) v <- b; 1 / v }
print(dead())
live <- function() { if (TRUE) a <- 0; b <- -0; for (i in 1:2) v <- b; 1 / v }
print(live())
# global code: rep() is a special called with the AST's -0, c() a builtin
# called with the compiled (folded, shared) -0
grep10 <- function() { a <- rep(0, 10); b <- rep(-0, 10); for (i in 1:2) v <- b; 1 / v[1] }
print(grep10())
gnamed <- function() { a <- c(x = 0); b <- c(y = -0); for (i in 1:2) v <- b; 1 / v }
print(gnamed())
arg <- function(x) { for (i in 1:2) NULL; x }
args0 <- function() { a <- 0; for (i in 1:2) v <- arg(-0); 1 / v }
print(args0())
# namespace code
ns <- function(f) { environment(f) <- .BaseNamespaceEnv; f }
ten <- ns(function() { a <- rep(0, 10); b <- rep(-0, 10); for (i in 1:2) v <- b; 1 / v[1] })
print(ten()); print(ten())
eleven <- ns(function() { a <- rep(0, 11); b <- rep(-0, 11); for (i in 1:2) v <- b; 1 / v[1] })
print(eleven()); print(eleven())
named <- ns(function() { a <- c(x = 0); b <- c(x = -0); for (i in 1:2) v <- b; 1 / v })
print(named()); print(named())
renamed <- ns(function() { a <- c(x = 0); b <- c(y = -0); for (i in 1:2) v <- b; 1 / v })
print(renamed()); print(renamed())
sq <- ns(function() { a <- 0; b <- sqrt(0) * -1; for (i in 1:2) v <- b; 1 / v })
print(sq()); print(sq())
# a top-level loop is compiled too
for (i in 1:2) { a <- 0; b <- -0 }
print(1 / b)
# a fold is evaluated when the code is compiled: an error it raises is the
# last error's message; a branch a folded test drops is never compiled; a
# warning is not signalled
.Internal(seterrmessage("none\n"))
errs <- function() { for (i in 1:2) if (i > 5) 1 + "a"; 1 }
print(errs())
cat(.Internal(geterrmessage()), "\n")
.Internal(seterrmessage("none\n"))
dropped <- function() { for (i in 1:2) if (TRUE) 1 else 1 + "a"; 2 }
print(dropped())
cat(.Internal(geterrmessage()))
.Internal(seterrmessage("none\n"))
dropped2 <- function() { for (i in 1:2) if (1 == 1) 1 else 1 + "a"; 2 }
print(dropped2())
cat(.Internal(geterrmessage()))
.Internal(seterrmessage("none\n"))
nestedif <- function() { x <- list(1:2); for (i in 1) x[[if (1 == 1) 1 else 1 + "a"]][1] <- 0L; x }
print(nestedif())
cat(.Internal(geterrmessage()))
warns <- function() { for (i in 1:2) if (i > 5) sqrt(-1); 3 }
print(warns())
runs <- function() { for (i in 1:2) NULL; sqrt(-1) }
print(runs())
print(tryCatch((function() { for (i in 1:2) NULL; 1 + "a" })(), error = conditionMessage))
