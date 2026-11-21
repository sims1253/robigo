# A name given as a string (`"x" <- v`, `missing("x")`) is installTrChar's
# symbol, and install's errors for it are raised at each evaluation: a
# zero-length name or one past MAXIDSIZE (names.c:install), and
# translateToNative's for a non-ASCII "bytes" name, in a closure the
# compiler compiles, called twice, and at the top level.
h <- function() { "" <- 1 }
for (i in 1:2) print(tryCatch(h(), error = function(e) conditionCall(e)))
tryCatch(h(), error = conditionMessage)
k <- function() missing("")
for (i in 1:2) print(tryCatch(k(), error = function(e) conditionMessage(e)))
s <- function() { "" <<- 1 }
tryCatch(s(), error = conditionMessage)
tryCatch("" <- 1, error = conditionMessage)
long <- strrep("a", 10001)
tryCatch(eval(substitute(L <- 1, list(L = long))), error = conditionMessage)
b <- "\xe9"
Encoding(b) <- "bytes"
tryCatch(eval(substitute(S <- 1, list(S = b))), error = conditionMessage)
bf <- eval(substitute(function() S <- 1, list(S = b)))
for (i in 1:2) print(tryCatch(bf(), error = function(e) conditionCall(e)))
bm <- eval(substitute(function() missing(S), list(S = b)))
tryCatch(bm(), error = conditionMessage)
a <- "abc"
Encoding(a) <- "bytes"
tryCatch(eval(substitute(S <- 1, list(S = a))), error = conditionMessage)
exists("abc")
m <- function(a) { "x" <- 2; "y" = 3; c(x, y, missing("a")) }
m()
m()
