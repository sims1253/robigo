# Integer arithmetic for every operand shape: NA, overflow warnings, recycling
a <- c(1L, NA, .Machine$integer.max, -.Machine$integer.max, 5L, -7L)
b <- c(2L, 3L, 1L, -1L, 0L, 2L)
for (op in c("+", "-", "*", "/", "^", "%%", "%/%")) { f <- match.fun(op); print(f(a, b)); print(f(a, 3L)); print(f(10L, a)); print(f(a, c(1L, 2L))) }
a * a
tryCatch(a + 1:4, warning = function(w) conditionMessage(w))
x <- 1:10; x - 1L; x * 2L; -x; integer(0) + 1L; 1:3 + integer(0)
.Machine$integer.max + 1L
