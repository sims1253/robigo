# Ops group dispatch asks chooseOpsMethod() when the operands select
# different methods (R: eval.c:DispatchGroup, R_chooseOpsMethod).
l <- structure(1L, class = "left")
r <- structure(2L, class = "right")
`+.left` <- function(e1, e2) "left"
`+.right` <- function(e1, e2) "right"

# neither chosen: the warning, and the internal operator
print(withCallingHandlers(l + r, warning = function(w) {
  print(conditionMessage(w))
  print(conditionCall(w))
  invokeRestart("muffleWarning")
}))

# left chosen (forward call)
chooseOpsMethod.left <- function(x, y, mx, my, cl, reverse) TRUE
l + r
r + l
rm(chooseOpsMethod.left)

# right chosen (reverse call)
chooseOpsMethod.right <- function(x, y, mx, my, cl, reverse) reverse
l + r
r + l
rm(chooseOpsMethod.right)

# the arguments the chooser receives
chooseOpsMethod.left <- function(x, y, mx, my, cl, reverse) {
  cat("chooser: x =", class(x), " y =", class(y), " reverse =", reverse, "\n")
  print(cl)
  print(identical(mx, `+.left`))
  print(identical(my, `+.right`))
  print(identical(my, `+.left`))
  reverse
}
r + l
l + r
rm(chooseOpsMethod.left)

# a chooser returning NULL counts as FALSE
chooseOpsMethod.left <- function(x, y, mx, my, cl, reverse) NULL
print(suppressWarnings(l + r))
chooseOpsMethod.left <- function(x, y, mx, my, cl, reverse) if (reverse) NULL else TRUE
l + r
rm(chooseOpsMethod.left)

# group methods and comparison operators
Ops.left <- function(e1, e2) paste("Ops.left", .Generic)
`==.right` <- function(e1, e2) "==.right"
chooseOpsMethod.left <- function(x, y, mx, my, cl, reverse) TRUE
l == r
r == l
l * r
rm(chooseOpsMethod.left, Ops.left, `==.right`)

# a class in a data frame's class vector
df <- data.frame(a = 1:2)
class(df) <- c("tbl", "data.frame")
s <- structure(10, class = "special")
`+.special` <- function(e1, e2) "special"
chooseOpsMethod.tbl <- function(x, y, mx, my, cl, reverse) TRUE
df + s
s + df
rm(chooseOpsMethod.tbl)
chooseOpsMethod.special <- function(x, y, mx, my, cl, reverse) TRUE
df + s
s + df
rm(chooseOpsMethod.special)

# a chooser for data.frame operands against another class
x <- structure(1, class = "foo")
Ops.foo <- function(e1, e2) "Ops.foo"
chooseOpsMethod.foo <- function(x, y, mx, my, cl, reverse) TRUE
x + data.frame(a = 1)
data.frame(a = 1) + x
x == data.frame(a = 1)

# errors from the chooser's result
chooseOpsMethod.foo <- function(x, y, mx, my, cl, reverse) NA
tryCatch(x + data.frame(a = 1), error = function(e) print(conditionCall(e)))
tryCatch(x + data.frame(a = 1), error = function(e) print(conditionMessage(e)))
chooseOpsMethod.foo <- function(x, y, mx, my, cl, reverse) c(TRUE, FALSE)
tryCatch(x + data.frame(a = 1), error = function(e) print(conditionMessage(e)))
chooseOpsMethod.foo <- function(x, y, mx, my, cl, reverse) "yes"
tryCatch(x + data.frame(a = 1), error = function(e) print(conditionMessage(e)))
chooseOpsMethod.foo <- function(x, y, mx, my, cl, reverse) "true"
x + data.frame(a = 1)
chooseOpsMethod.foo <- function(x, y, mx, my, cl, reverse) 1
x + data.frame(a = 1)
chooseOpsMethod.foo <- function(x, y, mx, my, cl, reverse) stop("boom")
tryCatch(x + data.frame(a = 1), error = function(e) print(conditionMessage(e)))

# the warning at top level
rm(chooseOpsMethod.foo)
x + data.frame(a = 1)
