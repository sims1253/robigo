# A code-generation error after a surrogate warning reaches a parseError
# calling handler (R: gram.y:raiseParseError signals a classed condition).
globalCallingHandlers(parseError = function(e) cat("handled\n"))
1 |> f("\uD800", _)
