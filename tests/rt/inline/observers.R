# Observers around and inside calls robigo may run inline (pure closures
# called from compiled code): the call stack, frames, promises, errors and
# warnings with their calls, redefinition mid-loop, laziness and depth.
# Every case runs in a compiled top-level loop, three times: the first
# calls compile the closures, the last reaches the inline hook. Each
# observer is also called through a pure wrapper, an inline candidate
# whose run gives up at the observer's call.
add1 <- function(x) x + 1
w_add1 <- function(x) add1(x)
for (k in 1:3) r <- tryCatch(c(add1(1), w_add1("a")), error = function(e) e)
print(conditionMessage(r)); print(conditionCall(r))
over <- function(x) x + 1L
w_over <- function(x) over(x)
for (k in 1:3) r <- withCallingHandlers(c(over(1L), w_over(.Machine$integer.max)), warning = function(w) {
  if (k == 3) { print(conditionMessage(w)); print(conditionCall(w)) }
  invokeRestart("muffleWarning")
})
print(r)
cond <- function(x) if (x > 0) "pos" else "neg"
w_cond <- function(x) cond(x)
for (k in 1:3) r <- tryCatch(c(cond(1), w_cond(NA)), error = function(e) e)
print(conditionMessage(r)); print(conditionCall(r))
sc <- function(x) sys.call()
sf <- function(x) identical(sys.function(), sf)
pf <- function() parent.frame()
mc <- function(alpha, beta) match.call()
ms <- function(x) missing(x)
su <- function(x) substitute(x)
en <- function(x) environment()
ox <- function(x) { on.exit(if (k == 3) cat("exit", x, "\n")); x + 1 }
w_sc <- function(y) sc(y + 1)
w_sf <- function(y) sf(y)
w_pf <- function() pf()
w_mc <- function(a) mc(a, b = 2)
w_ms <- function() ms()
w_su <- function(y) su(y * 2)
w_en <- function(y) en(y)
w_ox <- function(y) ox(y)
for (k in 1:3) {
  a <- add1(k)
  r <- list(sc(a + 1), w_sc(a), sf(1), w_sf(1),
            identical(pf(), environment()), identical(w_pf(), environment()),
            mc(1, b = 2), w_mc(1), c(ms(), ms(1), w_ms()), su(a * 2), w_su(a),
            get("x", envir = en(5)), get("x", envir = w_en(6)), ox(add1(2)), w_ox(3))
}
print(r)
f <- function(x) x + 1
s <- 0
for (i in 1:6) {
  if (i == 4) f <- function(x) x * 100
  s <- s + f(i)
}
print(s)
leaf <- function(x) x + 1
mid <- function(x) leaf(x) * 2
s <- 0
for (i in 1:6) {
  if (i == 4) leaf <- function(x) x - 1
  s <- s + mid(i)
}
print(s)
lazy <- function(x, y) x
side <- function(x, y) x + y
for (k in 1:3) r <- c(lazy(k, stop("never")), lazy(k, {cat("never\n"); 3}), side(k, {if (k == 3) cat("forced\n"); 2}))
print(r)
caller <- function(x) add1(x)
for (k in 1:3) r <- caller({if (k == 3) cat("promise\n"); 5})
print(r)
deep <- function(n) if (n == 0) 0 else deep(n - 1) + 1
for (k in 1:3) r <- deep(100)
print(r)
