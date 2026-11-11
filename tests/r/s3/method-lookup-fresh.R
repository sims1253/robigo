# S3 method lookup must see every change between calls: a method defined,
# shadowed or removed mid-loop, the S3 methods table, attach/detach, class
# changes; .Class/.Generic/.Group as R binds them, also for NextMethod and Ops.
f <- function(x, ...) UseMethod("f")
f.default <- function(x, ...) "default"
x <- structure(1, class = "k")

# Defined, redefined and removed mid-loop.
out <- character()
for (i in 1:5) {
  if (i == 2) f.k <- function(x, ...) "k1"
  if (i == 3) f.k <- function(x, ...) "k2"
  if (i == 4) rm(f.k)
  out <- c(out, f(x))
}
print(out)

# Shadowed in a frame between the call env and topenv, then unshadowed.
g <- function(shadow) {
  if (shadow) f.k <- function(x, ...) "local"
  f(x)
}
print(c(g(TRUE), g(FALSE), g(TRUE)))
h <- function() {
  r <- character()
  for (i in 1:3) {
    if (i == 2) f.k <- function(x, ...) "local2"
    if (i == 3) rm(f.k)
    r <- c(r, f(x))
  }
  r
}
print(h())
# A nested closure: the method is in the enclosing function's frame.
outer <- function() {
  f.k <- function(x, ...) "enclosing"
  inner <- function() f(x)
  inner()
}
print(outer())

# The S3 methods table: registered methods are found after the call env.
registerS3method("f", "k", function(x, ...) "registered")
print(f(x))
f.k <- function(x, ...) "global wins"
print(f(x))
rm(f.k)
print(f(x))
registerS3method("f", "k", function(x, ...) "re-registered")
print(f(x))
# A base generic: the method is in base's table, after the call env.
tk <- structure(1, class = "tk")
for (i in 1:3) {
  if (i == 2) registerS3method("rev", "tk", function(x) "rev.tk", envir = baseenv())
  if (i == 3) rev.tk <- function(x) "global rev.tk"
  print(rev(tk))
}
rm(rev.tk)

# attach/detach of an environment holding a method.
x2 <- structure(1, class = "att")
e <- new.env()
assign("f.att", function(x, ...) "attached", envir = e)
print(f(x2))
attach(e, name = "s3methods")
print(f(x2))
detach("s3methods")
print(f(x2))

# Class attribute changes mid-loop, and implicit classes.
y <- 1
r <- character()
for (i in 1:4) {
  if (i == 2) class(y) <- "k2"
  if (i == 3) f.k2 <- function(x, ...) "k2 method"
  if (i == 4) y <- unclass(y)
  r <- c(r, f(y))
}
print(r)
f.matrix <- function(x, ...) paste("matrix", .Class[1])
f.integer <- function(x, ...) paste(.Class, collapse = "/")
f.numeric <- function(x, ...) paste(.Class, collapse = "/")
f.function <- function(x, ...) "function"
print(c(f(matrix(1:4, 2)), f(1L), f(1.5), f(sum), f(function() 1)))
m <- matrix(1:4, 2)
for (i in 1:2) {
  if (i == 2) rm(f.matrix)
  print(f(m))
}

# .Class, .Generic, .Group as dispatch binds them.
p <- function(x, ...) UseMethod("p")
p.a <- function(x, ...) list(cls = .Class, gen = .Generic, grp = .Group, meth = .Method, nxt = NextMethod())
p.b <- function(x, ...) list(cls = .Class, gen = .Generic, meth = .Method)
z <- structure(1, class = c("zz", "a", "b"))
str1 <- p(z)
print(str1$cls)
print(str1$gen)
print(str1$grp)
print(str1$meth)
print(str1$nxt)
# .Class modified inside a method does not touch the object's class.
q <- function(x) UseMethod("q")
q.a <- function(x) { .Class[1] <- "changed"; .Class }
print(q(z))
print(class(z))

# A generic name given as a variable, and with attributes.
gn <- "f"
ff <- function(x) UseMethod(gn)
f.k3 <- function(x, ...) .Generic
print(ff(structure(1, class = "k3")))

# NextMethod chains that change mid-loop.
nm <- function(x) UseMethod("nm")
nm.a <- function(x) c("a", NextMethod())
nm.default <- function(x) "default"
w <- structure(1, class = c("a", "b"))
for (i in 1:3) {
  if (i == 2) nm.b <- function(x) c("b", NextMethod())
  if (i == 3) rm(nm.b)
  print(nm(w))
}

# Ops group methods: defined, a specific method added, removed.
v <- structure(2, class = "money")
for (i in 1:4) {
  if (i == 2) Ops.money <- function(e1, e2) paste("Ops", .Generic, .Group, paste(.Method, collapse = ","))
  if (i == 3) `+.money` <- function(e1, e2) paste("plus", .Generic, nchar(.Group), paste(.Class, collapse = ","))
  if (i == 4) rm(`+.money`)
  print(v + 1)
  print(1 + v)
}
rm(Ops.money)
print(unclass(v + 1))
vv <- structure(2, class = c("sub", "money"))
Ops.money <- function(e1, e2) paste(.Class, collapse = ",")
print(vv * vv)
local({
  Ops.money <- function(e1, e2) "local Ops"
  print(vv - 1)
})
print(vv - 1)

# .Class after NextMethod reaches the default method, and NA classes.
d <- function(x) UseMethod("d")
d.a <- function(x) NextMethod()
d.default <- function(x) .Class
print(d(structure(1, class = c("a", "b"))))
d.NA <- function(x) c("NA method", .Class)
na1 <- 1
oldClass(na1) <- c(NA, "a")
print(d(na1))
