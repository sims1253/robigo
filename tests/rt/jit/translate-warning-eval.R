# A name given as a string that does not translate to the native encoding
# (LC_ALL=C, the .locale sidecar): `"x" <- v` and `missing("x")` install it
# with escapes after installTrChar's warning "unable to translate", given at
# each evaluation as GNU R's interpreter gives it, against the function's
# call. Each closure is called once, so the JIT compiles none of them
# (jit/translate-warning-bc.R has the compiled ones).
options(warn = 1)
f <- function() { "\u4e2d" <- 1; ls() }
f()
fe <- function() { "\u4e2d" = 2; ls() }
fe()
fs <- function() { "\u4e2d" <<- 3; 0 }
fs()
get("<U+4E2D>")
g <- function() missing("\u4e2d")
tryCatch(g(), error = conditionMessage)
# the warning comes before the value is evaluated
fv <- function() { "\u4e2d" <- { cat("value\n"); 4 }; 0 }
fv()
# at the top level
"\u4e2d" <- 5
get("<U+4E2D>")
# options(warn = 2): the warning is an error, and nothing is assigned
fw <- function() { "\u4e2d" <- 6; ls() }
options(warn = 2)
tryCatch(fw(), error = function(e) print(conditionCall(e)))
options(warn = 1)
get("<U+4E2D>")
# a name that translates gives none
f3 <- function() { "abc" <- 7; ls() }
f3()
