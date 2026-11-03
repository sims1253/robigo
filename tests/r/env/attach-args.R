# R: envir.c:do_attach — asInteger (with its coercion warning) and
# isValidStringF, both plain error() against the calling closure.
cat(tryCatch(attach(list(x = 42), pos = "string"), error = conditionMessage), "\n")
cat(tryCatch(attach(list(), name = 42), error = conditionMessage), "\n")
e <- new.env(); e$a <- 1
attach(e, name = "myenv")
print(search()[[2]]); print(get("a")); detach("myenv")

# do_pos2env: coerceVector warns, then every position is validated.
cat(tryCatch(pos.to.env("hello"), error = conditionMessage), "\n")
cat(tryCatch(pos.to.env(-2), error = conditionMessage), "\n")
cat(tryCatch(pos.to.env(integer()), error = conditionMessage), "\n")
print(is.environment(tryCatch(pos.to.env(2), error = function(e) e)))

# R_lsInternal3 / do_env2list: plain errors; asLogical's NA is FALSE.
cat(tryCatch(.Internal(env2list(list(1, 2, 3), TRUE, TRUE)), error = conditionMessage), "\n")
print(.Internal(env2list(new.env(), "string", list())))
cat(tryCatch(.Internal(ls(42, TRUE, TRUE)), error = conditionMessage), "\n")
print(.Internal(ls(globalenv(), NA, TRUE)))
print(.Internal(env2list(new.env(), TRUE, NA)))

# do_diag: every non-numeric x goes through coerceVector(x, REALSXP).
print(diag("a", 3, 4))
print(diag(c("a", "b", "c"), 3, 4))
print(suppressWarnings(diag(c(TRUE, FALSE), 2, 2)))
print(diag(NULL, 0, 0))
