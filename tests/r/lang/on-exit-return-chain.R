# A return() in on.exit code makes its value the closure's returnValue
# (eval.c R_execClosure); the remaining on.exit code (context.c endcontext)
# sees it through returnValue().
f <- function() {
  on.exit(return(4))
  on.exit({ fret1 <<- returnValue(); return(5) }, add = TRUE)
  on.exit({ fret2 <<- returnValue(); x <<- 2 }, add = TRUE)
  3
}
for (i in 1:3) {
  res <- f()
  print(c(res, fret1, fret2, x))
}
# after an error the value is undefined until a return() sets it
g <- function() {
  on.exit(seen1 <<- returnValue("undefined"))
  on.exit(return("rescued"), add = TRUE)
  on.exit(seen2 <<- returnValue("undefined"), add = TRUE)
  stop("boom")
}
for (i in 1:3) {
  print(g())
  print(c(seen1, seen2))
}
# with after = FALSE the return() runs first
h <- function() {
  on.exit(seen3 <<- returnValue())
  on.exit(return(7), add = TRUE, after = FALSE)
  1
}
h()
seen3
