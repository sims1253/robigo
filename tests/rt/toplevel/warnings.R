# Deferred warnings print after the statement's value (PrintWarnings): one,
# several, and a call long enough to move the message to the next line.
x <- 2147483647L
x + 1L
f <- function(v) v + 1L
f(x)
g <- function() { x + 1L; x + 2L; x * 2L; 0 }
g()
h <- function(a_rather_long_argument_name, another_rather_long_name) a_rather_long_argument_name + another_rather_long_name
h(x, 1L)
"after"
