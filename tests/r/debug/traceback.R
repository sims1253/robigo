# R: errors.c:jump_to_top_ex, R_GetTracebackOnly, R_GetTraceback, do_traceback.
f <- function(x) g(x + 1)
g <- function(y) stop("boom ", y)

# nothing yet
traceback()
print(.traceback())

# not written by an error that is handled, or in a script with no reason
# to keep running
tryCatch(f(1), error = function(e) NULL)
print(.traceback())

options(catch.script.errors = TRUE)
f(1)
traceback()
x <- .traceback()
print(typeof(x))
print(length(x))
print(x)
print(typeof(.Traceback))
print(.Traceback[[1]])
print(class(.Traceback[[1]]))

# the next error replaces it, and one with no frames empties it
h <- function() f(2)
h()
traceback()
stop("at top level")
print(.traceback())

# errors from builtins, and from methods
k <- function(x) x + "a"
k(1)
traceback()
print.zz <- function(x, ...) stop("in a method")
p <- function() print(structure(1, class = "zz"))
p()
traceback()

# traceback(n) is the live stack, n frames skipped
m <- function(n) traceback(n)
m(0)
m(1)
m(99)
print(m(1))
outer <- function() inner()
inner <- function() .traceback(1)
print(outer())

# the skip must be valid; what is not an integer is taken for a traceback
print(tryCatch(.Internal(traceback(-1L)), error = function(e) conditionMessage(e)))
print(tryCatch(.Internal(traceback(NA_integer_)), error = function(e) conditionMessage(e)))
traceback(list("a", "b"))
traceback(-1)
print(traceback("not a stack"))

# max.lines
long <- function() stop("long")
call_with_long_args <- function(...) long()
call_with_long_args(aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa = 1, bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb = 2,
                    cccccccccccccccccccccccccccccc = 3, ddddddddddddddddddd = 4)
traceback()
traceback(max.lines = 1)
