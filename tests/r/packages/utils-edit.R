# edit() and fix() (R: main/edit.c:do_edit): the object is deparsed to a
# file, the editor runs on it, and the file is parsed and evaluated back.
library(utils)
err <- function(expr) tryCatch(expr, error = function(e) print(e))
options(editor = "true")
f <- function(x) x + 1
g <- edit(f); print(g)
print(edit(list(a = 1, b = "x")))
print(edit(1:3))
# NULL writes nothing: the session's default file is read back as it is.
print(edit(NULL))
print(edit(quote(a + b)))
print(edit(factor(c("u", "v"))))
print(edit(c(a = 1L, b = NA)))
# Errors: the editor's status, and the arguments do_edit checks.
err(edit(f, editor = "false"))
err(edit(f, editor = ""))
err(edit(f, editor = 1))
err(edit(f, file = 1))
# An editor that changes the file.
ed <- tempfile(fileext = ".sh")
writeLines(c("#!/bin/sh", "sed -i 's/+ 1/* 2/' \"$1\""), ed)
Sys.chmod(ed, "755")
g <- edit(f, editor = ed); print(g); print(g(5))
# A function gets back the original's environment.
env <- new.env()
local(k <- function() 1, env)
k2 <- edit(env$k)
print(identical(environment(k2), env))
# A syntax error in the edited file.
bad <- tempfile(fileext = ".sh")
writeLines(c("#!/bin/sh", "echo 'x <- ' > \"$1\"", "echo 'y y' >> \"$1\""), bad)
Sys.chmod(bad, "755")
err(edit(f, editor = bad))
# An explicit file; NULL with a file evaluates what is in it.
tf <- tempfile(fileext = ".R")
v <- edit(c(1.5, 2), file = tf); print(v); print(readLines(tf))
writeLines("a <- 3; a * 7", tf)
print(edit(file = tf))
print(a)
# fix() assigns the edited object in the global environment.
x <- c(a = 1L, b = 2L)
fix(x)
print(x)
fix(f, editor = ed)
print(f)
# With keep.source the result carries a srcref to the edited text.
options(keep.source = TRUE)
h <- edit(f)
print(h)
print(class(attr(h, "srcref")))
print(attr(h, "srcref"))
print(getSrcFilename(h))
options(keep.source = FALSE)
# The shell runs as `sh`, as system(3) starts it.
system("echo $0")
