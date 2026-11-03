# R: coerce.c:do_is — is.integer is TYPEOF == INTSXP *and* not a factor;
# is.numeric/is.matrix/is.array dispatch for classed objects; is.single
# checks arity before its constant error.
print(is.integer(as.factor(c(1, 2, 3))))
print(is.integer(factor(c("a", "b"))))
print(is.integer(1:3))
print(is.integer(as.integer(c(1.1, 2.2))))
print(is.numeric(as.factor(c(1, 2))))
cat(tryCatch(is.single(1), error = conditionMessage), "\n")
cat(tryCatch(is.single(), error = conditionMessage), "\n")
print(is.array(structure(1:2, class = "cls")))
is.array.cls <- function(x) 42
print(is.array(structure(1:2, class = "cls")))
is.matrix.cls <- function(x) 42
print(is.matrix(structure(1:2, class = "cls")))

# R: sysutils.c:do_setTimeLimit / do_setSessionTimeLimit — limits that are
# not finite and positive disable, NULL is returned (invisible).
print(setTimeLimit(cpu = FALSE, elapsed = Inf))
print(setTimeLimit(cpu = 10, elapsed = 5, transient = TRUE))
print(setSessionTimeLimit())
print(setTimeLimit())
