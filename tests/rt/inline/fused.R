# Inline runs take some instruction groups as one step (a formal compared
# with a scalar and branched on, a call of a function looked up by name, a
# formal returned, arithmetic returned). Where a group's precondition
# fails, the instructions run one by one, or the run gives up where they
# would: the values, errors and warnings, with their calls, are GNU R's.
# Each case runs three times from a compiled closure.
ci <- function(n) if (n < 2L) n else ci(n - 1L) + 1L
fc <- function(n) if (n < 2) n else fc(n - 1) + fc(n - 2)
ov <- function(n) if (n < 1L) n + 2147483647L else ov(n - 1L) + 0L
r1 <- function() c(ci(10L), fc(15), fc(0.5), fc(TRUE))
for (k in 1:3) print(r1())
cond <- function(e) list(conditionMessage(e), conditionCall(e))
r2 <- function() tryCatch(ci(1i), error = cond)
for (k in 1:3) print(r2())
r3 <- function() tryCatch(ci(NA_integer_), error = cond)
for (k in 1:3) print(r3())
r4 <- function() withCallingHandlers(ov(3L), warning = function(w) {
  print(conditionMessage(w)); print(conditionCall(w))
  invokeRestart("muffleWarning")
})
for (k in 1:3) print(r4())
r5 <- function() { s <- 0; for (i in 1:30) s <- s + fc(i %% 7); s }
for (k in 1:3) print(r5())
