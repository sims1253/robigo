# R: print.c:909 — a pairlist prints through printList and then, because the
# LISTSXP arm breaks out of the switch, its other attributes are printed too
# (printAttributes skips only attributes that are structural for the value:
# names for a non-array, dim/dimnames for a list).
x <- as.pairlist(c(1, 2, 3))
names(x) <- c("a", "b", "c")
attr(x, "myattr") <- 42
print(x)

# No names: the attribute is still printed, after the elements.
y <- pairlist(1, "two")
attr(y, "myattr") <- "tagged"
print(y)

# as.vector(mode = "pairlist") keeps the attributes the same way and prints
# them; mode = "any" returns the pairlist unchanged.
print(as.vector(x, mode = "pairlist"))
print(as.vector(x, mode = "any"))

# A parse() expression is an EXPRSXP, but as.pairlist() of one becomes a
# LISTSXP with the attribute retained.
z <- parse(text = "x; y; z")
names(z) <- c("a", "b", "c")
attr(z, "myattr") <- 42
print(as.vector(z, mode = "pairlist"))
print(as.vector(list(1, 2, 4), mode = "pairlist"))

# Structural attributes stay hidden: names, and a class drives dispatch.
w <- as.pairlist(letters[1:2])
attr(w, "class") <- "mypl"
print(w)
print(attributes(w))
