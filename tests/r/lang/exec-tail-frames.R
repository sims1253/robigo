# Tailcall/Exec in tail position replace the calling closure's frame
# (eval.c:do_tailcall's jump and handle_exec_continuation): the new call
# takes the old frame's place on the stack, its parent frame is the old
# frame, and deep recursion runs in constant stack.
g <- function(...) {
    print(sys.call())
    print(sys.nframe())
    print(identical(parent.frame(), .GlobalEnv))
    list(...)
}
f <- function(x) { e <- environment(); Tailcall(g, x, y = 2) }
print(f(1))
pf <- function(x) { e <- environment(); Tailcall(function() identical(parent.frame(), e)) }
pf()
f2 <- function(x) Tailcall("g", x)
print(f2(3))
f3 <- function(x) { on.exit(cat("bye\n")); Tailcall(g, x) }
print(f3(4))
f4 <- function() Exec(quote(g(5)))
print(f4())
h <- function() { z <- 9; environment() }
f5 <- function() Exec(quote(g(z)), h())
print(f5())
f6 <- function(n) if (n == 0) "done" else Tailcall(f6, n - 1)
f6(1e5)
f7 <- function() Tailcall(sum, 1, 2)
f7()
k <- function(n) { if (n == 0) return(sys.function()); Tailcall(k, n - 1) }
identical(k(3), k)
m <- function(n, acc = 0) if (n == 0) acc else Exec(call("m", n - 1, acc + n))
m(10000)
p <- function() Tailcall(function(a) sys.function(), 1)
class(p())
q1 <- function(x) { if (x) Tailcall(g, "a") else Tailcall(g, "b") }
print(q1(FALSE))
r1 <- function() { x <- 1; Tailcall(missingf, 1) }
try(r1())
s1 <- function() Exec(quote(sys.call()))
s1()
s3 <- function() Exec(quote(invisible(7)))
s3()
withVisible(s3())
v <- function(n) if (n == 0) stop("deep") else Tailcall(v, n - 1)
tryCatch(v(5e4), error = function(e) conditionMessage(e))
# exit code the function expression registers runs before the new call
ox <- function() Tailcall({ on.exit(cat("exit\n")); function() 3 })
ox()
# on.exit(NULL) clears the exit code, so the jump is taken; a call out of
# tail position still evaluates a string FUN as a call head
oc <- function() { on.exit(cat("never\n")); on.exit(NULL); Tailcall(g, "cleared") }
print(oc())
ns <- function() { x <- Tailcall("sum", 1, 2); x + 1 }
print(tryCatch(ns(), error = function(e) conditionMessage(e)))
