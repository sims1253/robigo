cat("start\n")
x <- tryCatch(1 %ÿ% 2, error = function(e) conditionMessage(e))
print(nchar(x, "bytes"))
