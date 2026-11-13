# invisible() and the top level's R_Visible.
invisible(5)
(invisible(6))
x <- invisible(7)
x
invisible()
f <- function() invisible(8)
f()
(f())
