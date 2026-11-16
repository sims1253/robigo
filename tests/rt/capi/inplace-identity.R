# A native observer of R's in-place writes (docs/g3.md §15.3 E5): C
# evaluates each write and compares the object bound before with the one
# bound after, holding the old one while it compares (rt_assign_copies:
# TRUE when the write copies). GNU R writes an object that only its binding
# references in place and copies a shared one; robigo's production answers
# are GNU R's. ROBIGO_NO_INPLACE=1 copies at every write, so this recording
# holds in production only (the .requires: inplace-identity).
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/identity.so"))
copies <- function(sym, e) .Call("rt_assign_copies", sym, e, globalenv())
# right after a top-level assignment: .Last.value holds the value without
# a count (tests/rt/toplevel/last-value-counts.R)
x <- c(1, 2, 3)
print(copies(quote(x), quote(x[2] <- 20)))
print(copies(quote(x), quote(x[[3]] <- 30)))
print(copies(quote(x), quote(names(x) <- c("a", "b", "c"))))
print(copies(quote(x), quote(attr(x, "u") <- 1)))
# shared: the write copies, the other binding keeps the old object (alive,
# so its address before and after can be compared)
y <- x
a <- .Call("rt_addr", y)
print(copies(quote(x), quote(x[1] <- 10)))
print(.Call("rt_addr", y) == a)
print(x)
print(y)
# nested: the outer list and the inner one written in place
l <- list(a = 1, b = list(c = 2))
print(copies(quote(l), quote(l$b$c <- 3)))
print(l$b$c)
# a local, in a closure's first call (the AST) and later ones (byte code).
# C cannot evaluate a compiled write, so this compares the addresses read
# before and after it: exact without the moving collector, which could
# move a copy to the address of an original that died (#709)
f <- function() {
  v <- c(1, 2)
  a <- .Call("rt_addr", v)
  v[1] <- 5
  c(.Call("rt_addr", v) == a, v)
}
print(f())
print(f())
print(f())
