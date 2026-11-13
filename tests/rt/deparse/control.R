# deparse2buff of the language specials: if/else (inside braces too),
# for, while, repeat, break, next, function, braces' indentation.
quote(if (a) b else c)
quote(if (a) if (b) c else d)
quote({ x <- 1; if (x > 0) { y <- 2 } else y <- 3; for (i in 1:10) next; while (TRUE) break; repeat { break } })
quote(function(x, y = 2) { x + y })
quote(function(x, ...) NULL)
quote(function(x) function(y) x + y)
quote(x <- function() NULL)
quote({
    if (a) {
        b
    } else if (c) {
        d
    } else {
        e
    }
})
quote({ { { { { { x } } } } } })
