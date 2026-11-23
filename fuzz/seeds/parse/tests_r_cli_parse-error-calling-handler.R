# A parse error is errorcall(): global calling handlers see its message
# and call before the default handler prints it.
globalCallingHandlers(error = function(e) {
  cat("handler:", conditionMessage(e), "| call:", deparse(conditionCall(e)), "\n")
})
f <- function(x) x
1 |> f(_)
