# The call of a warning or an error is deparse1s's first line: a long
# call is cut where deparse breaks it.
aaaaaaaaaaaaaaaaaaaaaaaaaa <- 1; bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb <- 2; ccccccccccccccccccccccccccc <- 3
x <- 1:3 + 1:2
y <- (aaaaaaaaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb + ccccccccccccccccccccccccccc) + 1:2 + 1:3
z <- if (TRUE) { 1:3 } + 1:2
(function(a, b) a + b)(1:3, 1:2)
