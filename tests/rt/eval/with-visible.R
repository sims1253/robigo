# .Internal(withVisible(x)) (eval.c do_withVisible): the value of x and
# R_Visible as its evaluation left it.
str1 <- function(r) cat(names(r), "|", deparse(r$value), "|", r$visible, "\n")
str1(withVisible(1))
str1(withVisible(invisible(2)))
str1(withVisible(x <- 3))
str1(withVisible((x <- 4)))
str1(withVisible(x))
str1(withVisible(NULL))
str1(withVisible(assign("y", 5)))
str1(withVisible(print))
withVisible(6)
w <- withVisible(7)
w
names(w)
typeof(w$visible)
# a closure: its last value's visibility
fi <- function() invisible(8)
fv <- function() 9
fa <- function() z <- 10
fp <- function() (z <- 11)
fr <- function() return(invisible(12))
fe <- function() { z <- 1; z }
str1(withVisible(fi()))
str1(withVisible(fv()))
str1(withVisible(fa()))
str1(withVisible(fp()))
str1(withVisible(fr()))
str1(withVisible(fe()))
# builtins: invisible ones (X = 1) and visible ones
str1(withVisible(invisible()))
str1(withVisible(gctorture(FALSE)))
str1(withVisible(c(1, 2)))
str1(withVisible(if (FALSE) 1))
str1(withVisible(for (i in 1:2) i))
str1(withVisible(suppressWarnings(invisible(13))))
# withVisible's argument is forced once, in the caller
cnt <- 0
str1(withVisible({ cnt <- cnt + 1; invisible(cnt) }))
cnt
# compiled callers (a loop makes the JIT compile them)
cmp <- function(k) {
  for (i in seq_len(2)) NULL
  switch(k,
    withVisible(invisible(14)),
    withVisible(15),
    withVisible(z <- 16),
    withVisible((z <- 17)),
    withVisible(fi()),
    withVisible(fv()))
}
for (k in 1:6) str1(cmp(k))
for (k in 1:6) str1(cmp(k))
# errors and warnings inside
r <- tryCatch(withVisible(stop("boom")), error = function(e) e)
conditionMessage(r)
conditionCall(r)
str1(withVisible(withCallingHandlers(
  { warning("careful"); invisible(18) },
  warning = function(w) { cat("warned:", conditionMessage(w), "\n"); invokeRestart("muffleWarning") })))
r <- tryCatch(.Internal(withVisible()), error = function(e) e)
conditionMessage(r)
r <- tryCatch(.Internal(withVisible(1, 2)), error = function(e) e)
conditionMessage(r)
r <- tryCatch(withVisible(), error = function(e) e)
conditionMessage(r)
conditionCall(r)
