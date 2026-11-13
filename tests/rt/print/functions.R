# A closure in a list: PrintClosure's deparse (keep.source = FALSE). A
# closure of the global environment prints no environment line.
f <- function(x, y = 2) {
    z <- x + y
    if (z > 3) z else -z
}
list(f)
list(g = function(a, ...) NULL, h = function() 1)
