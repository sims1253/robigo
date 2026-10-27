# Empty and missing subscripts, `...` forwarding, named arguments.
m <- 1:6
dim(m) <- c(2L, 3L)
pick <- function(i, j) m[i, j]
pick(1)
pick(, 2)
pick()
pick(2, c(1, 3))
fwd <- function(...) m[...]
fwd(2, 3)
fwd(, 1)
fwd(1, , drop = FALSE)
fwd2 <- function(x, ...) x[...]
fwd2(letters, 3:4)
fwd2(letters)
outer_fn <- function(k) inner_fn(k)
inner_fn <- function(i) (11:13)[i]
outer_fn()
outer_fn(2)
dflt <- function(i = 2) (11:13)[i]
dflt()
set_some <- function(v, i) {
  v[i] <- 0L
  v
}
set_some(1:3)
set_some(1:3, 3)
v <- 1:4
v[] <- 9L
v
# arguments given by name are positional for the default method
`[`(c(5, 6, 7), 2)
`[`(m, 2, 3)
`[[`(list(a = 1, b = 2), "b")
`$`(list(abc = 1), "abc")
`$`(list(abc = 1), abc)
`[<-`(1:3, 2, value = 0L)
`[[<-`(list(1), 2, value = "two")
`$<-`(list(), "nm", 1)
`[`(m, i = 1, j = 2)
tryCatch(`[`(drop = FALSE, x = m, 1, 2), error = function(e) conditionMessage(e))
# assignment through a replacement call with missing indices
w <- 1:6
dim(w) <- c(2L, 3L)
w[, 2] <- 0L
w[1, ] <- -1L
w
w[, ] <- 5L
w
