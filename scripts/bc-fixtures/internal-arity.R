# Byte-code translation cases: GNU R's compiled .Internal call with a
# count its checkArity refuses (CALLBUILTIN on the internal). GNU R
# evaluates the arguments, then raises the arity error against the call;
# the translation emits that error after the argument code.
cases <- list(
  list(name = "k_getenv0", fun = function() .Internal(Sys.getenv()),
       calls = "print(tryCatch(k_getenv0(), error = conditionMessage))"),
  list(name = "k_getenv0_call", fun = function() .Internal(Sys.getenv()),
       calls = "print(tryCatch(k_getenv0_call(), error = conditionCall))"),
  list(name = "k_getenv3", fun = function(x) .Internal(Sys.getenv(x, cat("evaluated\n"), 3)),
       calls = "print(tryCatch(k_getenv3('HOME'), error = conditionMessage))"),
  list(name = "k_paste0_ok", fun = function(x) .Internal(paste0(list(x), NULL)),
       calls = "print(k_paste0_ok('a'))")
)
