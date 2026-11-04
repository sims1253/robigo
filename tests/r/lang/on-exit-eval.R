# on.exit() inside eval() registers on eval's own context (eval.c:do_eval's
# CTXT_RETURN, cloenv = envir) and runs when eval() returns, before the code
# after it. withr::defer() and testthat's local_*() rely on this in
# test_that() blocks, which are eval(code, test_env).
e <- new.env()
eval(quote(do.call(base::on.exit, list(quote(cat("deferred\n")), TRUE, FALSE), envir = e)), e)
cat("after eval\n")

# the innermost context with that cloenv is eval's, not the closure's
f <- function() {
  eval(quote(on.exit(cat("eval exit\n"))), environment())
  cat("in f\n")
}
f()

local({
  on.exit(cat("local exit\n"))
  cat("in local\n")
})
cat("after local\n")

# add/after order, and the value and visibility of eval()
e2 <- new.env()
v <- eval(quote({
  on.exit(cat("one\n"))
  on.exit(cat("two\n"), add = TRUE)
  on.exit(cat("zero\n"), add = TRUE, after = FALSE)
  42
}), e2)
v
eval(quote({ on.exit(cat("exit\n")); invisible(7) }), new.env())
withVisible(eval(quote({ on.exit(cat("exit\n")); 8 }), new.env()))

# on an error unwinding through eval()
r <- tryCatch(eval(quote({ on.exit(cat("cleanup\n")); stop("boom") }), new.env()),
              error = function(e) conditionMessage(e))
r

# a mock-style restore: the binding is back once eval() returns
x <- "original"
env <- new.env()
eval(quote({
  old <- x
  do.call(on.exit, list(substitute(x <<- old, list(old = old)), TRUE), envir = env)
  x <<- "mocked"
  cat("inside:", x, "\n")
}), env)
cat("outside:", x, "\n")
