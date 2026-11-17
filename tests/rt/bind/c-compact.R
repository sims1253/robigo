# c(), unlist() and attributes<- over compact sequences (1:n, seq_len(n))
# nested in lists: the answers read their elements as GNU R's bind.c does,
# through INTEGER (AC1 materializes them before the walk)
unlist(list(1:3, 4:6))
c(list(1:3), 4)
unlist(list(a = seq_len(3)))
c(1:3, 4:6)
x <- 1:6
attributes(x) <- list(dim = 2:3)
attributes(x)
c(list(1:3, list(4:5)), recursive = TRUE)
unlist(list(a = 1:2, b = list(c = 3:4, d = 5L)))
unlist(list(1:3, 4:6), use.names = FALSE)
c(list(a = 1:2), b = 3:4)
c(list(1:3), list(seq_len(2)), recursive = TRUE)
unlist(list(list(1:2, list(3:4)), 5:6))
unlist(list(1:3, "a"))
unlist(list(1:2, c(1.5, 2.5)))
c(list(1:2), NULL, list(3:4))
w <- 1:3
attributes(w) <- list(names = letters[1:3], foo = 4:6)
attributes(w)
attr(w, "bar") <- 7:9
attr(w, "bar")
z <- 1:6
attributes(z) <- list(dim = c(2L, 3L), dimnames = list(1:2, 1:3))
attributes(z)
