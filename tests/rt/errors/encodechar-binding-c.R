# "no binding for \"%s\"" and "no active binding for \"%s\"" with
# EncodeChar(PRINTNAME(sym)) (envir.c:3775-4028)
# This copy runs under LC_ALL=C (its .locale sidecar).
show <- function(expr) tryCatch(withCallingHandlers(expr, warning = function(w) {
    print(conditionMessage(w)); invokeRestart("muffleWarning") }),
  error = function(e) print(conditionMessage(e)))
syms <- list(as.name(paste0("a", intToUtf8(10L), "b")), as.name(paste0("a", intToUtf8(9L), "b")),
             as.name(paste0("x", intToUtf8(1L), "y")), quote(`café`))
for (s in syms) {
  nm <- as.character(s)
  show(lockBinding(nm, globalenv()))
  show(unlockBinding(nm, globalenv()))
  show(bindingIsLocked(nm, globalenv()))
  show(bindingIsActive(nm, globalenv()))
  show(activeBindingFunction(nm, globalenv()))
  assign(nm, 1)
  show(activeBindingFunction(nm, globalenv()))
  rm(list = nm)
}
