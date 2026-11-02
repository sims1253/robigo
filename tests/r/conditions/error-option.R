# R: errors.c:jump_to_top_ex, options.c (`error`), main.c:check_session_exit.
# GNU R keeps running a script after an error when options(error=) is set.

# a function becomes a call of it; anything else but a call or an
# expression is refused
print(tryCatch(options(error = 1), error = function(e) conditionMessage(e)))
print(tryCatch(options(error = "x"), error = function(e) conditionMessage(e)))
print(tryCatch(options(error = as.name("x")), error = function(e) conditionMessage(e)))
options(error = function() cat("function handler\n"))
print(class(getOption("error")))
print(getOption("error"))
f <- function() g()
g <- function() stop("boom")
f()
cat("continued after a function handler\n")

# a handler that is a function sees the stack of the error
options(error = function() {
    print(sys.nframe())
    print(sys.calls())
    cat(geterrmessage())
})
f()

# a quoted handler runs in the global environment, at no frame
options(error = quote(cat("quoted handler, frame", sys.nframe(), "\n")))
f()

# an expression runs element by element
options(error = expression(cat("first\n"), cat("second\n")))
print(getOption("error"))
f()

# handled errors do not reach it
options(error = quote(cat("must not run\n")))
tryCatch(f(), error = function(e) cat("tryCatch\n"))
r <- try(f(), silent = TRUE)
withCallingHandlers(
    tryCatch(f(), error = function(e) cat("inner tryCatch\n")),
    error = function(e) cat("calling handler\n"))

# on.exit code runs after the handler, while unwinding
options(error = quote(cat("handler before on.exit\n")))
h <- function() {
    on.exit(cat("on.exit\n"))
    stop("from h")
}
h()

# warnings wait for the handler
options(error = quote({
    warning("raised in the handler")
    cat("handler with pending warnings\n")
}))
w <- function() {
    warning("before the error")
    stop("after the warning")
}
w()

# an error in the handler ends the handling, not the script
options(error = quote(stop("in the handler")))
f()
cat("continued after an error in the handler\n")
print(geterrmessage())
h()
cat("on.exit still ran\n")

# handlers that leave by a restart
options(error = quote(invokeRestart("out")))
print(withRestarts({ f(); "not reached" }, out = function() "left through a restart"))

# NULL unsets it
options(error = NULL)
print(getOption("error"))

# q() in the handler ends the session at once
options(error = quote(q(status = 3L, save = "no")))
h()
cat("not reached\n")
