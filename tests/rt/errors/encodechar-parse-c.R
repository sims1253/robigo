# The parser's repeatedFormal error: EncodeChar(PRINTNAME(_new)) in the
# message and the condition's value (gram.y:CheckFormalArgs)
# This copy runs under LC_ALL=C (its .locale sidecar).
show <- function(expr) tryCatch(withCallingHandlers(expr, warning = function(w) {
    print(conditionMessage(w)); invokeRestart("muffleWarning") }),
  error = function(e) print(conditionMessage(e)))
syms <- list(as.name(paste0("a", intToUtf8(10L), "b")), as.name(paste0("a", intToUtf8(9L), "b")),
             as.name(paste0("x", intToUtf8(1L), "y")), quote(`café`))
for (s in syms) {
  nm <- as.character(s)
  e <- tryCatch(parse(text = paste0("function(`", nm, "`, `", nm, "`) 1")), error = function(e) e)
  print(class(e))
  print(conditionMessage(e))
  print(e$value)
}
