# save() and saveToConn (saveload.c's do_saveToConn): the magic number and
# the variables' pairlist, each format and version, to a connection save
# opens or one already open; promises forced or not; the errors; and
# load(verbose = TRUE)'s trace (R_InitReadItemDepth).
f <- paste0(Sys.getenv("TMPDIR", "/tmp"), "/robigo-rt-save-", Sys.getpid())
msg <- function(expr) tryCatch({ force(expr); "no error" }, error = function(e) conditionMessage(e))
x <- c(a = 1.5, b = NA); y <- list(p = "q", r = 1:3); z <- quote(g(h))
# (the file's text for the ASCII formats; readBin is not in robigo-rt yet)
text <- function(file) { con <- file(file, "rb"); on.exit(close(con)); readChar(con, file.size(file), useBytes = TRUE) }
for (a in list(list(ascii = FALSE, version = 3), list(ascii = TRUE, version = 3),
               list(ascii = FALSE, version = 2), list(ascii = TRUE, version = 2))) {
    save(x, y, z, file = f, ascii = a$ascii, version = a$version, compress = FALSE)
    print(file.size(f))
    if (a$ascii) writeLines(text(f)) else print(substr(text(f), 1, 4))
    e <- new.env()
    print(load(f, envir = e))
    print(identical(mget(c("x", "y", "z"), envir = e), list(x = x, y = y, z = z)))
}
# compressed (gzip by default) and through list=
save(list = c("y", "x"), file = f)
print(file.size(f) < 150)
e <- new.env(); print(load(f, envir = e)); print(e$y)
# a connection already open stays open; one save opens is closed
con <- file(f, "wb")
save(x, file = con)
print(isOpen(con))
close(con)
e <- new.env(); print(load(f, envir = e)); print(e$x)
# the variables found from envir, promises forced unless eval.promises = FALSE
src <- new.env()
delayedAssign("lazy", { cat("forced\n"); 42 }, assign.env = src)
save(lazy, file = f, envir = src, eval.promises = FALSE)
e <- new.env(); print(load(f, envir = e))
print(typeof(.Internal(get("lazy", e, "any", FALSE))))
save(lazy, file = f, envir = src)
e <- new.env(); print(load(f, envir = e)); print(e$lazy)
# an active binding is looked up twice (do_saveToConn's two R_findVar);
# the second value is saved
n <- 0
makeActiveBinding("ab", function() { n <<- n + 1; n }, src)
save(ab, file = f, envir = src)
print(n)
e <- new.env(); print(load(f, envir = e)); print(e$ab)
# a version whose digit is a NUL: the magic number's C string ends before
# it, then R_Serialize refuses the version
con <- file(f, "wb")
print(msg(.Internal(saveToConn("x", con, FALSE, 208L, globalenv(), TRUE))))
close(con)
print(file.size(f))
# (serialize() to a connection streams too: the format line, then the refusal)
con <- file(f, "wb")
print(msg(serialize(1, con, version = 4L)))
close(con)
print(file.size(f))
# load(verbose = TRUE)
save(x, y, z, file = f, compress = FALSE)
print(load(f, envir = new.env(), verbose = TRUE))
nested <- list(list(1, list(u = 2)), w = pairlist(k = 3))
save(nested, file = f)
invisible(load(f, envir = new.env(), verbose = 2))
# the errors
print(msg(save(nosuch, file = f)))
con <- file(f, "wb")
print(msg(.Internal(saveToConn(1, con, FALSE, NULL, globalenv(), TRUE))))
print(msg(.Internal(saveToConn("x", con, FALSE, 1L, globalenv(), TRUE))))
print(msg(.Internal(saveToConn("x", con, FALSE, 0L, globalenv(), TRUE))))
print(msg(.Internal(saveToConn("x", con, FALSE, NA, globalenv(), TRUE))))
print(msg(.Internal(saveToConn("x", con, FALSE, NULL, list(), TRUE))))
print(msg(.Internal(saveToConn("x", con, FALSE, NULL, globalenv(), NA))))
print(msg(.Internal(saveToConn("x", con, NA, NULL, globalenv(), TRUE))))
r <- tryCatch(.Internal(saveToConn("x", con, NA, NULL, globalenv(), TRUE)), error = function(e) conditionCall(e))
print(r)
print(msg(.Internal(saveToConn("x", con, FALSE, 4L, globalenv(), TRUE))))
print(msg(.Internal(saveToConn("x", con, FALSE, NULL, NULL, TRUE))))
close(con)
con <- file(f, "w")
print(msg(.Internal(saveToConn("x", con, FALSE, NULL, globalenv(), TRUE))))
print(msg(save(x, file = con)))
print(msg(save(x, file = con, ascii = TRUE)))
close(con)
e <- new.env(); print(load(f, envir = e)); print(e$x)
con <- file(f, "r")
print(msg(.Internal(saveToConn("x", con, FALSE, NULL, globalenv(), TRUE))))
close(con)
print(msg(save(x, file = 1)))
invisible(file.remove(f))
