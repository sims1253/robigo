# prmatrix(): the old matrix printer
prmatrix(matrix(1:4, 2))
prmatrix(matrix(1:4, 2), rowlab = c("a", "b"), collab = c("x", "y"))
prmatrix(matrix(c("a", NA, "b", "c"), 2), quote = FALSE, right = TRUE, na.print = "-")
m <- matrix(1:4, 2, dimnames = list(R = c("a", "b"), C = c("x", "y")))
prmatrix(m)
x <- prmatrix(matrix(1:2, 1)); print(x)
tryCatch(prmatrix(matrix(1:4, 2), rowlab = 1:2), error = function(e) print(conditionMessage(e)))
tryCatch(prmatrix(matrix(1:4, 2), collab = 1:2), error = function(e) print(conditionMessage(e)))
tryCatch(prmatrix(matrix(1:4, 2), rowlab = "a"), error = function(e) print(conditionMessage(e)))
# right is an Rprt_adj: 2 centres the strings, 3 pads none
prmatrix(matrix(c("x", "abcde", "yy", "z"), 2), quote = FALSE, right = 2)
prmatrix(matrix(c("x", "abcde", "yy", "z"), 2), quote = FALSE, right = 3)
