# tryCmpfun (#964): what R code run inside a compile sees of it. A
# warning raised during the compile carries cmp.R's call, and a calling
# handler for it can read it. Run under LC_ALL=C.
f <- function() { "\u4e2d" <- 1; for (i in 1) NULL; 0 }
r <- withCallingHandlers(f(), warning = function(w) {
  print(conditionCall(w))
  print(conditionMessage(w))
  invokeRestart("muffleWarning")
})
print(r)
