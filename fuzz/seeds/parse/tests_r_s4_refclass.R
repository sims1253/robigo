# reference classes and classes extending environment
library(methods)
Acc <- suppressMessages(setRefClass("Acc", fields = list(bal = "numeric"),
  methods = list(
    dep = function(x) { bal <<- bal + x; invisible(.self) },
    show = function() cat("Bal:", bal, "\n"))))
a <- Acc$new(bal = 10)
a$dep(5)
a$bal
a
b <- a$copy()
b$dep(1)
c(a$bal, b$bal)
a$bal <- 1
a[["bal"]]
is(a, "envRefClass")
setClass("Mod", representation(pointer = "externalptr"), contains = "environment")
m <- new("Mod")
assign("v", 1, envir = m)
get("v", envir = m)
exists("v", envir = m)
ls(m)
f <- function() v
environment(f) <- m
f()
eval(quote(v + 1), m)
m$w <- 2
m[["w"]]
is.environment(m)
e2 <- new.env(parent = m)
get("v", envir = e2)
try(mget("v", envir = m))
