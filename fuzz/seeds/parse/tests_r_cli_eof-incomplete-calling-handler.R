# "unexpected end of input" is an error(): calling handlers see it.
globalCallingHandlers(error = function(e) cat("handler:", conditionMessage(e), "\n"))
x <- (
