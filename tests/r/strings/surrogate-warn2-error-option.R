# The converted warning is a top-level error: the `error` option runs and
# the session continues at the next line without evaluating the statement.
options(warn = 2, error = function() cat("error option ran\n"))
x <- "\uD800"
cat("after\n")
exists("x")
