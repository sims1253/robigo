# merge() and .Internal(merge): Shell-sorted index pairs
a <- data.frame(k = c(3, 1, 2, 2, 5, 1), v = letters[1:6])
b <- data.frame(k = c(2, 1, 1, 4, 2), w = LETTERS[1:5])
merge(a, b)
merge(a, b, all = TRUE)
merge(a, b, all.x = TRUE)
merge(a, b, by = NULL)[1:3, ]
merge(data.frame(k = 1:5000 %% 37, a = 1), data.frame(k = 5000:1 %% 41, b = 2))[c(1, 100, 1000), ]
x <- data.frame(id = c(1, 1, 2, 3, 3, 3), s = 1:6); y <- data.frame(id = c(3, 1, 3, 1), t = 11:14)
merge(x, y, sort = FALSE)
tryCatch(.Internal(merge(1:3, numeric(0), TRUE, FALSE)), error = function(e) conditionMessage(e))
tryCatch(.Internal(merge(1:3, 1:2, NA, FALSE)), error = function(e) conditionMessage(e))
