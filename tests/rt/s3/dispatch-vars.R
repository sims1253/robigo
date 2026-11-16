# S3 dispatch's variables in the method's frame (objects.c's createS3Vars,
# envir.c's addMissingVarsToNewEnv): they list first, in createS3Vars'
# order, before the formals; the body's locals list before them. Seen
# through ls(all.names = TRUE) (sorted and not), as.list(environment()),
# names() of sys.frame(), and in the frames of NextMethod's method, a group
# method and an internal generic's method; a formal or a local named as a
# variable; rm() of one; many methods (each method's frame shape is
# memoized); method frames kept and recycled in loops.
area <- function(s, ...) UseMethod("area")
area.sq <- function(s, ...) {
  zz <- 1
  B <- 2
  print(ls(all.names = TRUE))
  print(ls(all.names = TRUE, sorted = FALSE))
  print(ls())
  l <- as.list(environment(), all.names = TRUE)
  print(names(l))
  print(l[c(".Generic", ".Class", ".Method", ".Group")])
  print(identical(l$.GenericCallEnv, globalenv()))
  print(identical(l$.GenericDefEnv, globalenv()))
  print(names(as.list(environment(), all.names = TRUE, sorted = TRUE)))
  print(names(as.list(environment())))
  print(names(sys.frame(sys.nframe())))
  print(identical(sys.frame(sys.nframe()), environment()))
  NextMethod()
}
area.default <- function(s, ...) {
  print(ls(all.names = TRUE, sorted = FALSE))
  print(.Class)
  print(.Method)
  print(.Generic)
  0
}
sq <- structure(list(side = 2), class = "sq")
print(area(sq, 1))

# a formal named as a dispatch variable keeps its binding; a local named as
# one is that variable, assigned in place
peek <- function(x, ...) UseMethod("peek")
peek.a <- function(x, .Group = "mine") {
  print(ls(all.names = TRUE, sorted = FALSE))
  print(.Group)
  .Method <- "changed"
  print(ls(all.names = TRUE, sorted = FALSE))
  print(.Method)
  invisible()
}
peek(structure(1, class = "a"))

# rm() of a variable, then defined again: it lists first
rmv <- function(x) UseMethod("rmv")
rmv.z <- function(x) {
  rm(list = ".Generic")
  print(exists(".Generic", inherits = FALSE))
  print(ls(all.names = TRUE, sorted = FALSE))
  .Generic <- "again"
  print(ls(all.names = TRUE, sorted = FALSE))
  invisible()
}
rmv(structure(1, class = "z"))

# a group method's and an internal generic's method
Ops.money <- function(e1, e2) {
  print(ls(all.names = TRUE, sorted = FALSE))
  print(c(.Generic, .Group))
  print(.Method)
  1
}
m <- structure(1, class = "money")
print(m + m)
length.money <- function(x) {
  print(ls(all.names = TRUE, sorted = FALSE))
  print(c(.Generic, .Group, .Method))
  7L
}
print(length(m))

# method frames in loops: recycled between calls, kept when returned, and
# none of the variables when the method is called directly
ns <- function(x, ...) UseMethod("ns")
ns.k <- function(x, ...) {
  v <- x
  ls(all.names = TRUE, sorted = FALSE)
}
k <- structure(1, class = "k")
for (i in 1:5) r1 <- ns(k)
print(r1)
print(ns.k(k))
for (i in 1:3) r2 <- ns(k)
print(r2)
ns.e <- function(x, ...) {
  v <- x[[1]]
  environment()
}
keep <- list()
for (i in 1:4) keep[[i]] <- ns(structure(list(i), class = "e"))
for (i in 1:5) r3 <- ns(k)
for (f in keep) print(c(get("v", f), get(".Method", f), ls(envir = f, all.names = TRUE, sorted = FALSE)))

# many methods, each with its own frame shape, called in turn twice
many <- function(x, ...) UseMethod("many")
mk <- function(i) function(x, ...) {
  w <- i
  c(.Method, ls(all.names = TRUE, sorted = FALSE)[1:3])
}
for (i in 1:20) assign(paste0("many.c", i), mk(i))
for (r in 1:2) {
  for (i in c(1:20, 20:1)) {
    v <- many(structure(i, class = paste0("c", i)))
    if (r == 2 && i %% 7 == 0) print(v)
  }
}
