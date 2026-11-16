# Arguments whose code is a variable, or a variable and a scalar or a second
# variable under one operator: an inline run computes them directly, as the
# code runs. Integer overflow, NA, a string operand and division by zero
# give GNU R's values, errors and warnings, with their calls. Each case runs
# three times from a compiled closure.
id <- function(x) x
add <- function(x) x + 1
a1 <- function() { s <- 0; for (i in 1:20) s <- s + add(i) + add(2 * i) + add(i + i) + id(i - 1); s }
for (k in 1:3) print(a1())
a2 <- function() withCallingHandlers({ s <- 0L; for (i in 1:2) s <- s + add(i + 2147483647L); s },
  warning = function(w) { print(conditionMessage(w)); print(conditionCall(w)); invokeRestart("muffleWarning") })
for (k in 1:3) print(a2())
a3 <- function() { s <- 0; for (i in 1:5) s <- s + add(NA_real_ * i); s }
for (k in 1:3) print(a3())
a4 <- function() tryCatch({ v <- "a"; add(v + 1) }, error = function(e) list(conditionMessage(e), conditionCall(e)))
for (k in 1:3) print(a4())
a5 <- function() { s <- 0; for (i in 1:5) s <- s + add(i / 0) + add(-i %/% 2L); s }
for (k in 1:3) print(a5())
