# options(warn = 2): a warning becomes an error whose message keeps the
# warning's native bytes (vwarningcall_dflt passes its buffer to errorcall)
x <- "a\xffb"
options(warn = 2)
m <- tryCatch(warning(x), error = conditionMessage)
print(identical(m, paste0("(converted from warning) ", x)))
f <- function() warning(x)
r <- tryCatch(f(), error = function(e) e)
print(identical(conditionMessage(r), paste0("(converted from warning) ", x)))
print(conditionCall(r))
f()
