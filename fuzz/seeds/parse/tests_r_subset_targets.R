# Unusual assignment targets: NULL, active and locked bindings.
err <- function(expr) tryCatch(expr, error = function(e) conditionMessage(e))
x <- NULL; err(x[1] <- quote(f(a)))
x <- NULL; err(x[1] <- pairlist(1))
x <- NULL; err(x[1] <- new.env())
x <- NULL; err(x[1] <- function() 1)
x <- NULL; err(x[[1]] <- quote(f(a)))
x
x <- NULL; err(x$a <- quote(f(a)))
x
makeActiveBinding("ab", local({v <- 1:3; function(x) if (missing(x)) v else v <<- x}), environment())
ab[2] <- 10L
ab
ab[[3]] <- 30L
ab
lockBinding("lk", local({ lk <- 1:3; environment() }))
e <- new.env()
assign("z", 1:3, envir = e)
lockBinding("z", e)
r <- tryCatch(with_e <- local(z[1] <- 5L, envir = e), error = function(err) conditionMessage(err))
r
get("z", envir = e)
