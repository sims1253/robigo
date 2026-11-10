# A failed replacement keeps the binding exactly as it was: the
# attribute setters hand their (unchanged) object back through the
# restore channel (fast_replace re-binds it), and attributes<- installs
# on a duplicate (R: attrib.c do_attributesgets, "as an error later on
# would leave 'obj' changed"), so a mid-loop failure leaves the
# original attributes pristine.
q <- quote(a)
try(attr(q, "tsp") <- c(1, 2, 0))
print(q)
x <- quote(a)
try(class(x) <- "foo")
print(x)
y <- 1
try(attr(y, "tsp") <- c(1, 2, 0))
print(y)
z <- 1
try(names(z) <- c("a", "b"))
print(z)
w <- structure(1, old = 42)
try(attributes(w) <- list(dim = 5, comment = "x"))
print(attributes(w))
print(w)
print(attr(w, "old"))
