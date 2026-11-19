# The top-level parse's warnings: none from the trial parse of an
# incomplete statement's integer literals, options(warn = 1) prints at
# once, warn = 2 makes the parse warning an error.
x <- c(1.0L,
 2)
x
y <- 1.5L; warning("w2")
z <- c(1e-3L,
 2.0L,
 3)
options(warn = 1)
u <- 1.0L
options(warn = 2)
v <- tryCatch(parse(text = "1.0L"), error = function(e) conditionMessage(e)); v
w <- 1.0L
cat("not reached\n")
