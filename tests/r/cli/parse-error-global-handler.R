# The console's parser signals raiseParseError's parseError condition
# through the global calling handlers (R: gram.y:raiseParseError ->
# R_signalErrorCondition).
globalCallingHandlers(parseError = function(e) cat("handled", class(e)[1], e$lineno, e$colno, "\n"))
options(catch.script.errors = TRUE)
1 |> f("\uD800", _)
"\q"
x y
1 + 1
