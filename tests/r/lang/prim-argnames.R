# GNU R checks a supplied argument's name against a primitive's formals
# (matchArguments_NR, R 4.5.0 without the reference-list fast path): a name
# that matches no formal is an error naming both, and one matching a second
# formal again after a partial match is the multiple-formals error. Named
# calls that do match run: the checks are also validation of the common
# shapes.
p <- function(expr) {
  r <- tryCatch(expr, error = identity)
  if (inherits(r, "condition")) print(paste("ERROR:", conditionMessage(r))) else print(r)
}

# switch: the EXPR formal (p(quote(a=1)) errors "supplied argument name
# 'a' does not match 'EXPR'", the R-level closure doing it for a= and x=:
p(switch(a = 1, 2))
p(switch(1, "a"))
p(switch(x = 1, 2))
p(switch(EXPR = 2, "two"))
# identical(i = TRUE, 1, 2): 'i' partial-matches ignore.bytecode and
# ignore.environment: argument 1 matches multiple formal arguments.
p(identical(i = TRUE, 1, 2))
# missing(): formals of enforce the exact name.
p(missing(xx = 1))
p(missing(y = 1))
p(missing(x = 1))
p(missing(oh = 1))
# other primitive formals: is.na/is.invisible/invisible/length/names/dim/
# unclass/levels/pairs (all formals 'x', one wrong name errors naming it).
p(is.na(c(1, NA)))
p(is.na(z = 1))
p(invisible(x = 1))
p(invisible(i = 1))
p(length(x = 1))
p(length(l = 1))
p(names(x = c(a = 1)))
p(names(n = c(a = 1)))
p(dim(x = matrix(1, 2, 2)))
p(dim(d = matrix(1, 2, 2)))
p(unclass(x = 1))
p(unclass(u = 1))
p(levels(x = 1))
p(levels(lev = 1))
# values after the checks are intact: is.na(x=) and friends keep names.
cy <- c(1, NA)
print(is.na(cy))
cat(is.infinite(x = Inf), "\n")
