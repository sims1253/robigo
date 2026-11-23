# The lexer's warnings reach the global calling handlers, once from the
# trial parse and once from the real one.
globalCallingHandlers(warning = function(w) cat("handler:", conditionMessage(w), "\n"))
x <- "\uD800"
cat("after\n")
