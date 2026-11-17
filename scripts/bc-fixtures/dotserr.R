# Byte-code translation cases: `...` evaluated as a value where there is
# none (cmpSym's DOTSERR), as the value, inside an expression and inside an
# argument (scripts/bc-fixtures.R).
cases <- list(
  list(name = "k_dots_value", fun = function() ..., calls = "print(tryCatch(k_dots_value(), error = conditionMessage))"),
  list(name = "k_dots_assign", fun = function(x) { y <- ...; y },
       calls = "print(tryCatch(k_dots_assign(1), error = function(e) deparse(conditionCall(e))))"),
  list(name = "k_dots_arg", fun = function(x) identity(...),
       calls = "print(tryCatch(k_dots_arg(1), error = conditionMessage))"),
  list(name = "k_dots_error", fun = function() ..., calls = "k_dots_error()")
)
