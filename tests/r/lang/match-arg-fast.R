# match.arg(arg) with the choices taken from the caller's default: robigo
# answers the plain cases without running match.arg's R code. Every case
# here must behave as GNU R's match.arg does.
f <- function(type = c("linear", "quadratic", "cubic")) match.arg(type)
f()
print(f())
print(f("quad"))
print(f("cubic"))
print(f(NULL))
print(f(c("linear", "quadratic", "cubic")))
print(tryCatch(f("z"), error = conditionMessage))
print(tryCatch(f(""), error = conditionMessage))
print(tryCatch(f(NA_character_), error = conditionMessage))
print(tryCatch(f(c("linear", "cubic")), error = conditionMessage))
print(tryCatch(f(1), error = conditionMessage))
print(tryCatch(f(factor("linear")), error = conditionMessage))
print(f(c(a = "cubic")))
print(tryCatch(f(c(a = "linear", b = "quadratic", c = "cubic")), error = conditionMessage))

# Ambiguous partial matches, duplicates, exact before partial.
g <- function(m = c("aa", "ab", "a")) match.arg(m)
print(g("a"))
print(tryCatch((function(m = c("aa", "ab")) match.arg(m))("a"), error = conditionMessage))
print((function(m = c("x", "x", "y")) match.arg(m))("x"))

# The argument through a variable, a promise with side effects, a value
# assigned in the frame.
h <- function(type = c("one", "two")) match.arg(type)
v <- "tw"
print(h(v))
print(h({ cat("forcing\n"); "one" }))
k <- function(type = c("one", "two")) { type <- "two"; match.arg(type) }
print(k())
k2 <- function(type = c("one", "two")) { force(type); match.arg(type) }
print(k2())

# Defaults that are not a plain c() of strings.
print((function(type = c(A = "a", B = "b")) match.arg(type))())
print((function(type = letters[1:3]) match.arg(type))("b"))
print((function(type = toupper(c("a", "b"))) match.arg(type))())
print((function(type = c("été", "hiver")) match.arg(type))("é"))
print(tryCatch((function(type) match.arg(type))(), error = conditionMessage))

# A `c` other than base's, in the frame and in the global environment.
lc <- function(type = c("a", "b")) { c <- function(...) stop("my c"); match.arg(type) }
print(tryCatch(lc(), error = conditionMessage))
c <- function(...) base::c(..., "zz")
gc1 <- function(type = c("a", "b")) match.arg(type)
print(gc1())
print(gc1("zz"))
rm(c)
print(gc1())

# Another match.arg.
local({
  match.arg <- function(arg, choices, several.ok = FALSE) "mine"
  print((function(type = c("a", "b")) match.arg(type))())
})

# The symbol is not a formal of the calling function.
outer <- function(type = c("a", "b")) { inner <- function() match.arg(type); inner() }
print(tryCatch(outer(), error = conditionMessage))
print(tryCatch(outer("b"), error = conditionMessage))

# A formal called `arg`, the argument named, S3 methods, eval().
print((function(arg = c("x", "y")) match.arg(arg))("y"))
print((function(type = c("x", "y")) match.arg(arg = type))("y"))
style <- function(x, ...) UseMethod("style")
style.foo <- function(x, how = c("plain", "fancy"), ...) match.arg(how)
print(style(structure(1, class = "foo")))
print(style(structure(1, class = "foo"), how = "fan"))
ev <- function(type = c("p", "q")) eval(quote(match.arg(type)))
print(ev("q"))
ev2 <- function(type = c("p", "q")) evalq(match.arg(type), environment())
print(ev2())

# Errors report match.arg's call.
e <- tryCatch(f("nope"), error = function(e) e)
print(conditionCall(e))
e <- tryCatch(f(2), error = function(e) e)
print(conditionCall(e))

# Visibility, and base functions built on it.
f("lin")
x <- c(3, 1, 2, 2)
print(rank(x, ties.method = "min"))
print(rank(x, ties.method = "first"))
set.seed(3); print(rank(x, ties.method = "random"))
print(order(x, decreasing = TRUE, method = "radix"))
print(sort(x, method = "quick"))
print(tryCatch(rank(x, ties.method = "bogus"), error = conditionMessage))
for (i in 1:3) print(f(c("linear", "quadratic", "cubic")[i]))
