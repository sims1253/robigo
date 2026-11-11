dyn.load("altrep_lazy_string.so")
lazy <- function(fn) .Call("lazy_new", fn)

# A replacement reads a deferred package ALTREP operand while its target is
# still bound (eval.c:applydefine): R code the class's materialization runs,
# or a handler of the condition it signals, sees the target (#550).

# a `[<-` subscript
v <- c(p = 1, q = 2)
v[lazy(function() { print(v); "q" })] <- 3
print(v)

# a `[[<-` subscript
v[[lazy(function() { print(v); "p" })]] <- 4
print(v)

# the value `[<-` stores into a character vector
s <- c("a", "b")
s[2L] <- lazy(function() { print(s); "z" })
print(s)

# the value of a builtin replacement
n <- c(1, 2)
names(n) <- lazy(function() { print(n); c("x", "y") })
print(n)

# a nested replacement: the root is bound
a <- list(b = c(p = 1, q = 2))
a$b[lazy(function() { print(a); "p" })] <- 0
print(a)
a$b[[lazy(function() { print(a); "q" })]] <- 5
print(a)

# a calling handler of the error the materialization raises
v <- c(p = 1, q = 2)
r <- tryCatch(withCallingHandlers(
    v[lazy(function() stop("index boom"))] <- 9,
    error = function(e) print(v)),
    error = function(e) conditionMessage(e))
print(r)
print(v)
a <- list(b = c(p = 1, q = 2))
r <- tryCatch(withCallingHandlers(
    a$b[[lazy(function() stop("index boom"))]] <- 9,
    error = function(e) print(a)),
    error = function(e) conditionMessage(e))
print(r)
print(a)

# a loop: the target is the one the earlier iterations left
v <- c(p = 0, q = 0)
for (k in 1:2) v[lazy(function() { print(v); "q" })] <- k
print(v)
