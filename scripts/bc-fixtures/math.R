# Byte-code translation cases: bcEval's scalar math (MATH1, SQRT, EXP, LOG,
# LOGBASE): a number's value from the C function (-0 kept), "NaNs
# produced" against the call, integers, NAs, logicals and vectors through
# the builtin (scripts/bc-fixtures.R).
cases <- list(
  list(name = "k_sin", fun = function(x) 1 / sin(x),
       calls = c("print(k_sin(-0))", "print(k_sin(0))", "print(k_sin(c(-0, 0)))")),
  list(name = "k_math1s", fun = function(x) c(atan(x), sinh(x), expm1(x), log1p(x), tanpi(x), floor(x), sign(x)),
       calls = c("print(1 / k_math1s(-0))", "print(k_math1s(0.5))", "print(k_math1s(2L))")),
  list(name = "k_sqrt", fun = function(x) sqrt(x),
       calls = c("print(1 / sqrt(-0))", "print(k_sqrt(-0))", "print(k_sqrt(4L))", "print(k_sqrt(NA_integer_))",
                 "print(k_sqrt(TRUE))", "print(k_sqrt(NaN))")),
  list(name = "k_exp", fun = function(x) exp(x), calls = c("print(k_exp(1L))", "print(k_exp(-Inf))", "print(k_exp(FALSE))")),
  list(name = "k_log", fun = function(x) log(x),
       calls = c("print(k_log(-0))", "print(k_log(8L))", "print(k_log(NA_integer_))", "print(k_log(NaN))")),
  list(name = "k_logbase", fun = function(x, b) log(x, b),
       calls = c("print(k_logbase(8, 2))", "print(k_logbase(100L, 10))", "print(k_logbase(NaN, 2))",
                 "print(k_logbase(2, NA))", "print(k_logbase(c(1, 8), 2))")),
  list(name = "k_nan_warn", fun = function(x) { a <- sqrt(x); b <- gamma(0 * x); c <- log(x, 2); c(a, b, c) },
       calls = c("r <- withCallingHandlers(k_nan_warn(-1), warning = function(w) { print(conditionCall(w)); invokeRestart('muffleWarning') })",
                 "print(r)")),
  list(name = "k_lgl", fun = function(x) c(floor(x), log(x), sqrt(x)),
       calls = "print(k_lgl(TRUE))"),
  # nmath's own warnings (ML_WARNING) on a number, not NaN: against the call
  list(name = "k_gamma_range", fun = function(x) gamma(x),
       calls = c("r <- withCallingHandlers(k_gamma_range(1e-310), warning = function(w) { print(conditionMessage(w)); print(conditionCall(w)); invokeRestart('muffleWarning') })",
                 "print(r)", "print(k_gamma_range(1e-310))"))
)
