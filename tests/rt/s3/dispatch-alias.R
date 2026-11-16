# An S3 method's arguments are its generic's promises (objects.c's
# dispatchMethod passes cptr->promargs). robigo-rt binds them as aliases of
# the generic's argument slots until something needs the promise: these
# are the paths that must still see GNU R's one shared promise.

# the generic's binding rebound while the method runs: the method keeps the
# original argument (`<<-` from a closure made in the generic, `assign` and
# `rm` into the generic's frame), before and after its first read
gen <- function(x) { hook <<- function() x <<- 99; UseMethod("gen") }
gen.default <- function(x) { hook(); x }
print(gen(1))
gen2 <- function(x) UseMethod("gen2")
gen2.default <- function(x) { assign("x", 42, envir = sys.frame(-1)); x }
print(gen2(2))
gen3 <- function(x) UseMethod("gen3")
gen3.default <- function(x) { rm("x", envir = sys.frame(-1)); x }
print(gen3(3))
gen4 <- function(x) { hook <<- function() x <<- 99; UseMethod("gen4") }
gen4.default <- function(x) { x; hook(); x }
print(gen4(4))
gen5 <- function(x) { force(x); hook <<- function() x <<- 99; UseMethod("gen5") }
gen5.default <- function(x) { hook(); x }
print(gen5(5))
# ... and the generic sees its own new binding
gen6 <- function(x) { on.exit(print(x)); hook <<- function() x <<- 99; UseMethod("gen6") }
gen6.default <- function(x) { hook(); x + 1 }
print(gen6(6))
gen7 <- function(x) UseMethod("gen7")
gen7.default <- function(x) { eval(quote(x <- "eval"), sys.frame(-1)); c(x, get("x", envir = sys.frame(-1))) }
print(gen7("arg"))

# the method's own binding rebound or removed: the generic's is kept
own <- function(x) { on.exit(print(x)); UseMethod("own") }
own.default <- function(x) { x <- "method's"; x }
print(own("generic's"))
own2 <- function(x) { on.exit(print(x)); UseMethod("own2") }
own2.default <- function(x) { rm(x); exists("x", inherits = FALSE) }
print(own2("kept"))
own3 <- function(x) UseMethod("own3")
own3.default <- function(x) { f <- function() x <<- "set by <<-"; f(); x }
print(own3("arg"))

# a method's frame that outlives its call holds the promise: forced once,
# shared with the generic's frame (dispatch is on `d`: `x` is not forced)
lazy <- function(d, x) { gframe <<- environment(); UseMethod("lazy") }
lazy.default <- function(d, x) function() x
h <- lazy(0, { cat("forced\n"); 7 })
cat("before\n")
print(h())
print(h())
print(gframe$x)
eager <- function(x) UseMethod("eager")
eager.default <- function(x) { x; function() x }
k <- eager(8)
print(k())

# a method left by an error: its frame keeps the promise
boom <- function(d, x) UseMethod("boom")
boom.default <- function(d, x) { kept <<- environment(); stop("boom") }
print(tryCatch(boom(0, { cat("forced late\n"); 9 }), error = function(e) conditionMessage(e)))
print(kept$x)

# missing() and substitute() through the shared promise
mi <- function(x, y) UseMethod("mi")
mi.default <- function(x, y) c(missing(x), missing(y))
print(mi(1))
w <- function(a, b) mi(a, b)
print(w(1))
sb <- function(d, x) UseMethod("sb")
sb.default <- function(d, x) substitute(x)
print(sb(0, a + b))

# the binding as an object: the frame listed, the `...` list, NextMethod,
# a renamed match
ls1 <- function(x, y) UseMethod("ls1")
ls1.default <- function(x, y) { e <- as.list(environment()); list(sort(names(e)), e$x, e$y) }
str_list <- ls1(10, "b")
print(str_list)
dd <- function(x, y) UseMethod("dd")
dd.default <- function(...) list(...)
print(dd(1, y = { cat("y\n"); 2 }))
dd2 <- function(x, y) UseMethod("dd2")
dd2.default <- function(x, ...) c(x, ..1, ...length())
print(dd2(1, 2))
nm <- function(x) UseMethod("nm")
nm.a <- function(x) { x; NextMethod() }
nm.default <- function(x) unclass(x) * 2
print(nm(structure(3, class = "a")))
rn <- function(a, b) UseMethod("rn")
rn.default <- function(b, a) c(a = a, b = b)
print(rn(1, 2))

# the order arguments are forced in
fo <- function(x, y) UseMethod("fo")
fo.default <- function(x, y) { cat("in method\n"); y; x; invisible(NULL) }
fo({ cat("x forced\n"); 1 }, { cat("y forced\n"); 2 })

# a method that is a generic itself, and deep recursion through methods
mg <- function(x) UseMethod("mg")
mg.foo <- function(x) UseMethod("mg2")
mg2 <- function(x) "mg2 generic"
mg2.foo <- function(x) c("mg2.foo", unclass(x))
print(mg(structure("v", class = "foo")))
rec <- function(n) UseMethod("rec")
rec.default <- function(n) if (n == 0) 0 else 1 + rec(n - 1)
print(rec(300))
