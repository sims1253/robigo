# R_MissingArgError_c's condition classes for log's missing 'x'
cl <- function(expr) tryCatch(expr, error = function(e) print(class(e)))
cl(log())
cl(log(base = 2))
