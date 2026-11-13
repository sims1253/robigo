# A statement that warns, then fails: the error, then "In addition:" and
# the warning.
f <- function() { 2147483647L + 1L; missing_variable }
f()
