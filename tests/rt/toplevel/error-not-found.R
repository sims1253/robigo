# An error ends the script: what ran before it printed, the error is
# reported against its call, nothing after it runs.
1
f <- function() missing_variable
f()
2
