# R: envir.c:topenv — topenv(envir, matchThisEnv) also stops at matchThisEnv.
# sys.source() puts its envir in options(topLevelEnvironment), so code such as
# methods::setClass() (where = topenv(parent.frame())) defines classes there
# instead of in the namespace it is sourced from.
e <- new.env(parent = asNamespace("stats"))
print(identical(topenv(e, e), e))
print(identical(topenv(e), asNamespace("stats")))
print(identical(topenv(e, NULL), asNamespace("stats")))
# The walk stops at R_EmptyEnv without testing it against matchThisEnv, so
# an empty-env matchThisEnv gives the global environment (envir.c:topenv).
print(identical(topenv(new.env(parent = emptyenv()), emptyenv()), globalenv()))
print(identical(topenv(emptyenv(), emptyenv()), globalenv()))
print(identical(topenv(emptyenv()), globalenv()))

f <- tempfile(fileext = ".R")
writeLines(c(
  "g <- function() topenv(parent.frame())",
  "print(identical(g(), environment()))"
), f)
sys.source(f, envir = e)
unlink(f)
print(exists("g", envir = e, inherits = FALSE))
cat("ok\n")
