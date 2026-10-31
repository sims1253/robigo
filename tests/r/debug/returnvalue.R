# returnValue(): the value the enclosing function is about to return, while
# its on.exit code (or an exit tracer installed by trace()) runs. Outside
# exit code there is no pending value and the default is returned.
# R: eval.c:do_returnValue, context.c:endcontext (R_ExitContext).

note <- function(tag, v) cat(tag, ":", paste(deparse(v), collapse = " "), "\n")

# In the body there is no exit context: the default comes back.
f <- function(x) {
    note("body", returnValue("<default>"))
    x + 1
}
note("explicit return", f(2))

# Explicit return.
g <- function() {
    on.exit(note("on.exit explicit", returnValue("<default>")))
    return(99)
}
note("g()", g())

# Implicit last value.
h <- function() {
    on.exit(note("on.exit implicit", returnValue("<default>")))
    42
}
note("h()", h())

# Invisible return: on.exit sees the value, the call stays invisible.
inv <- function() {
    on.exit(note("on.exit invisible", returnValue("<default>")))
    invisible(7)
}
w <- withVisible(inv())
cat("withVisible:", w$visible, w$value, "\n")

# returnValue() is itself visible.
vis <- function() {
    on.exit({
        w <- withVisible(returnValue(0))
        cat("returnValue visible:", w$visible, w$value, "\n")
    })
    3
}
vis()

# A call made from exit code runs in the outer frame's exit context.
nested <- function() {
    inner <- function() note("inner body from on.exit", returnValue("<default>"))
    on.exit(inner())
    5
}
note("nested()", nested())

# A nested function's own on.exit sees its own return value.
nested2 <- function() {
    inner <- function() {
        on.exit(note("inner on.exit", returnValue("<default>")))
        6
    }
    on.exit(note("outer on.exit", returnValue("<default>")))
    inner()
    5
}
note("nested2()", nested2())

# An error unwinds the frame with no return value.
err <- function() {
    on.exit(note("error unwind", returnValue("<default>")))
    stop("boom")
}
try(err(), silent = TRUE)

# The default is used, and is evaluated by the wrapper before the internal.
err2 <- function() {
    on.exit(note("error unwind default", returnValue("fallback")))
    stop("boom")
}
try(err2(), silent = TRUE)

# No exit context at top level either.
note("top level", returnValue("<default>"))

# An exit tracer installed by trace() reads the same value.
library(methods)
t <- function(x) x + 1
trace(t, exit = quote(note("tracer", returnValue("<default>"))), where = environment())
t(2)
untrace(t, where = environment())
