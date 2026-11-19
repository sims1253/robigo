# The trial parse's warnings (an unpaired surrogate): printed before the
# next line of an incomplete statement, and "In addition:" to a parse error.
x <- c("\ud800",
 cat("second line\n"))
y <- "\ud800" +* 1
