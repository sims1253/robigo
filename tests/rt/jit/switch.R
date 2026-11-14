# cmp.R's switch handler in compiled code (GNU R 4.6.2): EXPR is the
# first argument whatever its tag; a string selects the first alternative
# of that name exactly, falling through empty ones (NA matches the name
# "NA", which the interpreted switch never does), else the unnamed
# default; a number selects by position, an empty alternative is an
# error; without a match the value is an invisible NULL. The JIT compiles
# a global closure without a loop at its second call, so each is called
# three times.
f <- function(x) switch(x, a = "A", b = , c = "BC", "other")
for (v in c("a", "b", "c", "d", NA)) for (i in 1:3) print(f(v))
g <- function(x) switch(x, a = 1, b = 2)
g("z")
g("z")
g("z")
for (i in 1:3) print(is.null(g("q")))
h <- function(n) switch(n, "one", "two", , "four")
for (i in 1:3) print(c(h(1), h(2), h(2.9), h(4)))
for (i in 1:3) print(is.null(h(0)))
for (i in 1:3) print(is.null(h(5)))
for (i in 1:3) print(tryCatch(h(3), error = function(e) conditionMessage(e)))
k <- function(x) switch(x, "NA" = "na-tag", "x")
for (i in 1:3) print(c(k(NA_character_), k("NA")))
t <- function() switch(a = "b", EXPR = "a", b = "B")
for (i in 1:3) print(tryCatch(t(), error = conditionMessage))
one <- function(x) switch(x, 1)
for (i in 1:3) print(c(one("a"), one(1)))
fall <- function(x) switch(x, a = , b = "AB", c = , d = )
for (v in c("a", "b", "c", "d", "e")) for (i in 1:3) print(fall(v))
s <- function(x) switch(x, "first", "second")
for (i in 1:3) print(s(2.5))
for (i in 1:3) print(tryCatch(s("a"), error = function(e) conditionMessage(e)))
none <- function(x) switch(x)
for (i in 1:3) print(none(1))
for (i in 1:3) print(none("a"))
len <- function(x) switch(x, 1, 2)
for (i in 1:3) print(tryCatch(len(1:2), error = function(e) conditionMessage(e)))
for (i in 1:3) print(tryCatch(len(NULL), error = function(e) conditionMessage(e)))
lst <- function(x) switch(x, a = 1, 2)
for (i in 1:3) print(is.null(lst(list("a"))))
sw <- function(type) { r <- 0; for (i in 1:50) r <- r + switch(type, add = i, sub = -i, 0); r }
print(c(sw("add"), sw("sub"), sw("none")))
vis <- function(x) switch(x, a = invisible(5), b = 6)
vis("a")
vis("a")
vis("a")
vis("b")
nest <- function(x, y) switch(x, a = switch(y, p = "ap", "a?"), b = "b")
for (i in 1:3) print(c(nest("a", "p"), nest("a", "q"), nest("b", "p")))
side <- function() { cat("EXPR evaluated\n"); "b" }
w <- function() switch(side(), a = 1, b = 2)
for (i in 1:3) print(w())
# an inlined wrapper in an alternative makes no context of its own
sv <- function(x, m) switch(x, a = is.vector(1, m), "none")
for (i in 1:3) print(sv("b", 1))
sv("a", 1)
