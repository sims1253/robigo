# Locks and active bindings: GNU R's messages, in hashed and closure frames.
msg <- function(expr) tryCatch({ force(expr); "no error" }, error = function(e) conditionMessage(e))
e <- new.env(); assign("x", 1, envir = e); lockBinding("x", e)
print(msg(assign("x", 2, envir = e)))
f <- function(y) { lockBinding("y", environment()); y <- 2 }
print(msg(f(1)))
lockEnvironment(e)
print(msg(assign("z", 2, envir = e)))
print(msg(rm("x", envir = e)))
print(msg(bindingIsLocked("zz", e)))
g <- function() { r <- 1; makeActiveBinding("r", function() 1, environment()) }
print(msg(g()))
h <- function() {
  makeActiveBinding("act", function(v) if (missing(v)) 42 else cat("set", v, "\n"), environment())
  print(act)
  act <- 7
  lockBinding("act", environment())
  print(msg(makeActiveBinding("act", function() 2, environment())))
  print(msg(act <- 8))
}
h()
print(msg(assign("x", 1, envir = emptyenv())))
print(msg(rm("c", envir = baseenv())))
print(msg(nonexistentfunction()))
print(msg((function(f) f())()))
