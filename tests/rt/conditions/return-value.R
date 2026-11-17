# eval.c:do_returnValue: in on.exit code, the value the function returns;
# the default when it returns none (an error) or outside on.exit code.
f <- function() { on.exit(print(returnValue())); 42 }
print(f())
g <- function() { on.exit(print(returnValue("default"))); stop("x") }
r <- try(g(), silent = TRUE)
h <- function() { on.exit(cat("value:", returnValue(), "\n")); invisible(7) }
h()
k <- function() { on.exit(print(returnValue())); return(c(a = 1)); 99 }
print(k())
m <- function() {
    on.exit(cat("first:", returnValue(), "\n"))
    on.exit(cat("second:", returnValue(), "\n"), add = TRUE)
    "two"
}
print(m())
inner <- function() returnValue("inner default")
o <- function() { on.exit(print(inner())); "outer" }
print(o())
print(returnValue("top level"))
p <- function() { on.exit({ v <- returnValue(); v[1] <- 0; print(v) }); x <- c(5, 6); x }
print(p())
q <- function() { on.exit(print(returnValue())); NULL }
print(q())
