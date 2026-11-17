# Strings made by paste, sprintf and coercion, then hashed where GNU R
# compares CHARSXPs by pointer (match, %in%, unique, duplicated,
# anyDuplicated, pmatch): every GNU R CHARSXP is cached, so equal strings
# are one object. D10 (docs/g3.md §15.2) runs robigo-rt both ways.
p <- paste0("k", 1:5)
q <- sprintf("k%d", 5:1)
print(match(p, q))
print(match(q, p))
print(p %in% c("k2", "k9"))
print(unique(c(p, q)))
print(.Internal(duplicated(c(p, q), FALSE, FALSE, NA)))
print(anyDuplicated(c(paste0("a", 1:3), "a2")))
n <- as.character(c(1, 2, 1, 2.5))
print(unique(n))
print(match("2", n))
print(match(c(2, 2.5), n))
u <- paste0("été", 1:2)
print(match(c("été1", "x"), u))
print(unique(c(u, "été2")))
tab <- paste0("e", 1:150)
print(pmatch(sprintf("e%d", c(150, 3, 77)), tab))
print(match(paste0("k", 2), p, incomparables = "k3"))
print(unique(c(sprintf("%s", "z"), "z"), incomparables = "y"))
