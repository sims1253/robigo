# installTrChar in the compiler and in substitute's list environment: a new
# symbol's PRINTNAME is the name's CHARSXP when no translation is needed
# (names.c:installNoTrChar), so a UTF-8 name keeps its mark. Each name is
# one no earlier code has made a symbol of.
enc <- function(s) Encoding(as.character(as.name(s)))
# a compiled assignment to a string
f <- compiler::cmpfun(function() { "zqé1" <- 1; environment() })
e <- f()
print(Encoding(ls(e)))
print(enc("zqé1"))
# a compiled missing() of a string
g <- compiler::cmpfun(function(a) missing("zqé2"))
print(tryCatch(g(1), error = conditionMessage))
print(enc("zqé2"))
# substitute()'s environment of a list
# (the name given at run time: a parsed tag would make the symbol first)
l <- list(x = 2, 3)
names(l)[2] <- "zqé3"
s <- substitute(x + 1, l)
print(s)
print(enc("zqé3"))
