# An unhandled error invokes the innermost "abort" restart
# (errors.c:jump_to_top_ex ends in try_jump_to_restart), which is how
# source(catch.aborts = TRUE) continues with the next expression.
tc <- textConnection('1:3
 2 + "3"
 cat(" .. in spite of error: happily continuing! ..\n")
 6*7')
r <- source(tc, catch.aborts = TRUE)
stopifnot(identical(r, list(value = 42, visible = TRUE)))

# the restart is popped, so a later error escapes again
f <- function() withRestarts(stop("boom"), abort = function() "caught")
print(f())
g <- function() { f(); stop("later") }
print(tryCatch(g(), error = function(e) conditionMessage(e)))

# nested abort restarts: the innermost one takes the error
h <- function()
    withRestarts(withRestarts(stop("inner"), abort = function() "outer"),
                 abort = function() "inner")
print(h())

# tryRestart and browser restarts would also be invoked; a differently
# named restart is left alone (the error still escapes)
print(tryCatch(withRestarts(stop("x"), other = function() "no"), error = function(e) "escaped"))
