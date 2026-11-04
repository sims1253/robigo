# on.exit() called directly in code that eval() runs registers in eval's
# own context, and runs when eval returns (R: eval.c:do_eval, endcontext).
# testthat's test_that() bodies run this way: their on.exit clean-up
# (lifecycle's env_unbind of a warning id) must run before the next test.
eval(quote({ on.exit(cat("exit 1\n")); cat("body 1\n") }), new.env())
cat("after 1\n")

f <- function() {
  e <- new.env()
  eval(quote({ on.exit(cat("exit 2\n")); cat("body 2\n") }), e)
  cat("f after eval\n")
}
f()

# local() is eval() underneath
x <- local({ on.exit(cat("exit 3\n")); cat("body 3\n"); 3 })
print(x)

# add = TRUE, after = FALSE, and the order they run in
eval(quote({
  on.exit(cat("a\n"))
  on.exit(cat("b\n"), add = TRUE)
  on.exit(cat("c\n"), add = TRUE, after = FALSE)
  cat("body 4\n")
}), new.env())

# the value, and its visibility, are the evaluated code's
eval(quote({ on.exit(cat("exit 5\n")); invisible(5) }), new.env())
print(withVisible(eval(quote({ on.exit(cat("exit 6\n")); 6 }), new.env())))

# return() in the on.exit code replaces the value
print(eval(quote({ on.exit(return(7)); 70 }), new.env()))

# return() in the body: the on.exit code still runs
print(eval(quote({ on.exit(cat("exit 8\n")); return(8); cat("not here\n") }), new.env()))

# an error unwinds through eval: its on.exit code runs before the handler
r <- tryCatch(
  eval(quote({ on.exit(cat("exit 9\n")); stop("boom") }), new.env()),
  error = function(e) { cat("caught:", conditionMessage(e), "\n"); 9 }
)
print(r)

# sys.on.exit() inside eval sees eval's own on.exit code
eval(quote({ on.exit(cat("exit 10\n")); print(sys.on.exit()) }), new.env())

# evalq and an expression vector
evalq({ on.exit(cat("exit 11\n")); cat("body 11\n") }, new.env())
eval(expression(on.exit(cat("exit 12\n")), cat("body 12\n")), new.env())
cat("done\n")
