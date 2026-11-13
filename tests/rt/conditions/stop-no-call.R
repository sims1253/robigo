f <- function() stop("message without call", call. = FALSE)
r <- tryCatch(f(), error = function(e) conditionCall(e))
print(r)
e <- simpleError("custom error")
r <- tryCatch(stop(e), error = function(err) conditionMessage(err))
r
f()
