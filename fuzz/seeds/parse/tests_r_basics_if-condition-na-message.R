# GNU R's asLogicalNoNA (eval.c): an NA condition that is not logical is
# "argument is not interpretable as logical", a logical NA "missing value
# where TRUE/FALSE needed".
f <- function(x) if (x) 1 else 2
for (x in list(NA, NA_integer_, NA_real_, NaN, NA_character_, NA_complex_))
  print(tryCatch(f(x), error = function(e) conditionMessage(e)))
print(tryCatch(while (NA_integer_) 1, error = function(e) conditionMessage(e)))
