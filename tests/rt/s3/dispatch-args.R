# The generic's arguments in its method: objects.c's dispatchMethod hands
# applyMethod cptr->promargs, the generic's own promises. An argument with a
# side effect is evaluated once, whoever forces it first: the generic before
# UseMethod, the method, both, the generic's on.exit code after the method
# returned, or NextMethod's method; everyone sees the same value. Also the
# dispatch object itself, `...` arguments, a forced argument passed on in
# the method's `...`, substitute() and missing() in the method, an unused
# argument's message, and a generic that overwrote its argument before
# UseMethod (the method gets the original promise).
count <- 0
side <- function(v) {
  count <<- count + 1
  cat("  forced", v, "\n")
  v
}
obj <- structure(1, class = "foo")
check <- function(label, value) cat(label, ":", value, "| evaluations", count, "\n")

# forced in the generic only, then read by the method
g1 <- function(x, y) {
  force(y)
  UseMethod("g1")
}
g1.foo <- function(x, y) "method ignores y"
count <- 0; check("generic only", g1(obj, side(1)))
g2 <- function(x, y) {
  y
  UseMethod("g2")
}
g2.foo <- function(x, y) y + 10
count <- 0; check("generic, then method", g2(obj, side(2)))

# forced in the method only
g3 <- function(x, y) UseMethod("g3")
g3.foo <- function(x, y) y * 2
count <- 0; check("method only", g3(obj, side(3)))
count <- 0; check("method only, in a loop", { s <- 0; for (i in 1:3) s <- s + g3(obj, side(i)); s })

# the generic's on.exit code after the method: forced by the method, or
# first there
g4 <- function(x, y) {
  on.exit(cat("  on.exit sees", y, "\n"))
  UseMethod("g4")
}
g4.foo <- function(x, y) y
count <- 0; check("method, then on.exit", g4(obj, side(4)))
g5 <- function(x, y) {
  on.exit(cat("  on.exit sees", y, "\n"))
  UseMethod("g5")
}
g5.foo <- function(x, y) "method skips y"
count <- 0; check("on.exit only", g5(obj, side(5)))
g6 <- function(x, y) {
  force(y)
  on.exit(cat("  on.exit sees", y, "\n"))
  UseMethod("g6")
}
g6.foo <- function(x, y) y
count <- 0; check("generic, method and on.exit", g6(obj, side(6)))

# through NextMethod: forced by the next method only, by the method first,
# by the generic first
g7 <- function(x, y) UseMethod("g7")
g7.foo <- function(x, y) NextMethod()
g7.default <- function(x, y) y
count <- 0; check("next method only", g7(obj, side(7)))
g8 <- function(x, y) UseMethod("g8")
g8.foo <- function(x, y) {
  y
  NextMethod()
}
g8.default <- function(x, y) y
count <- 0; check("method, then next method", g8(obj, side(8)))
g9 <- function(x, y) {
  y
  UseMethod("g9")
}
g9.foo <- function(x, y) NextMethod()
g9.default <- function(x, y) y
count <- 0; check("generic, then next method", g9(obj, side(9)))

# the dispatch object, evaluated once by UseMethod; `...` arguments
g10 <- function(x, ...) UseMethod("g10")
g10.foo <- function(x, ...) paste(class(x), ..1, ...length())
mk <- function() {
  count <<- count + 1
  cat("  made\n")
  obj
}
count <- 0; check("dispatch object and ...", g10(mk(), side("a"), side("b")))
count <- 0; check("dispatch object in a loop", { for (i in 1:3) r <- g10(mk(), i); r })

# a forced argument passed on in the method's `...`
g11 <- function(x, y) {
  y
  UseMethod("g11")
}
g11.foo <- function(x, ...) ..1 + 100
count <- 0; check("forced, into the method's ...", g11(obj, side(11)))

# substitute() and missing() in the method
g12 <- function(x, y) {
  y
  UseMethod("g12")
}
g12.foo <- function(x, y) identical(substitute(y), quote(side(12)))
count <- 0; check("substitute, forced", g12(obj, side(12)))
g13 <- function(x, y) UseMethod("g13")
g13.foo <- function(x, y) c(missing(y), identical(substitute(y), quote(side(13))))
count <- 0; check("substitute, delayed", g13(obj, side(13)))
check("missing", g13(obj))

# an unused argument: the message deparses its expression
g14 <- function(x, y) {
  y
  UseMethod("g14")
}
g14.foo <- function(x) 1
count <- 0; check("unused", tryCatch(g14(obj, side(14)), error = function(e) conditionMessage(e)))
g15 <- function(x, ...) UseMethod("g15")
g15.foo <- function(x) 1
check("unused, delayed", tryCatch(g15(obj, side(15)), error = function(e) conditionMessage(e)))

# a generic that overwrote its argument before UseMethod: the method gets
# the original promise, and UseMethod dispatches on it
g16 <- function(x, y) {
  x <- "overwritten"
  y <- "overwritten too"
  UseMethod("g16")
}
g16.foo <- function(x, y) paste(class(x), y)
count <- 0; check("overwritten in the generic", g16(obj, side(16)))

# a partial match in the method's call warns, and the warning's handler
# overwrites the generic's argument: the method still gets the original
# promise (GNU R's list was made before the match)
pg <- function(x) {
  gen_env <<- environment()
  UseMethod("pg")
}
pg.foo <- function(xyz) paste(class(xyz), xyz)
old <- options(warnPartialMatchArgs = TRUE)
r <- withCallingHandlers(pg(x = obj), warning = function(w) {
  cat("  warning:", conditionMessage(w), "\n")
  assign("x", "changed", envir = gen_env)
  invokeRestart("muffleWarning")
})
options(old)
check("partial match, handler overwrites", r)
