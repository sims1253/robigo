# Equal strings made separately (paste, sprintf, coercion), compared as
# GNU R compares cached CHARSXPs: ==, != (compiled and inlined), and the
# orderings, whose "same CHARSXP" shortcut skips collation.
g <- function(a) { r <- logical(3); for (i in 1:3) r[i] <- paste0("a", i) == a; r }
print(g("a2")); print(g("a2"))
h <- function(x) x == "b1"
k <- function() { r <- logical(2); for (i in 1:2) r[i] <- h(paste0("b", i)); r }
print(k()); print(k())
ne <- function() { r <- logical(2); for (i in 1:2) r[i] <- sprintf("c%d", i) != "c1"; r }
print(ne()); print(ne())
o <- c(paste0("z", 1), "z1", paste0("y", 2), as.character(1))
print(order(o))
print(sort(o))
print(rank(o))
print(min(o)); print(max(o))
print(pmin(c(paste0("m", 1), "m2"), c("m1", paste0("m", 2))))
print(pmax(c(paste0("m", 1), "m2"), c("m1", paste0("m", 2))))
print(is.unsorted(c(paste0("w", 1), "w1")))
print(order(c(paste0("g", 2), "g1", "g2", paste0("g", 1)), method = "radix"))
print(xtfrm(c(paste0("x", 1), "x1", "x0")))
