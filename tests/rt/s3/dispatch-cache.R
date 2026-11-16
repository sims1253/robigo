# The dispatch cache (usemethod's methods by generic, class vector and defining
# environment): methods defined, redefined and removed mid-loop at every level
# of R_LookupMethod's search, the generic's environment and the class vector changed.
area <- function(s) UseMethod("area")
area.default <- function(s) "default"
x <- structure(1, class = c("sq", "shape"))
# the top environment (global), from the top level
r <- character(0)
for (i in 1:6) {
  if (i == 2) area.shape <- function(s) "shape v1"
  if (i == 3) area.sq <- function(s) "sq v1"
  if (i == 4) area.sq <- function(s) "sq v2"
  if (i == 5) rm(area.sq)
  if (i == 6) rm(area.shape)
  r[i] <- area(x)
}
r
# the calling frame
f <- function() {
  r <- character(0)
  for (i in 1:5) {
    if (i == 2) area.sq <- function(s) "sq local"
    if (i == 3) area.sq <- function(s) "sq local v2"
    if (i == 4) area.sq <- 1
    if (i == 5) rm(area.sq)
    r[i] <- area(x)
  }
  r
}
f()
f()
# a frame between the caller and the top environment
g <- function() {
  h <- function() area(x)
  r <- character(0)
  for (i in 1:4) {
    if (i == 2) area.sq <- function(s) "sq in g"
    if (i == 3) area.shape <- function(s) "shape in g"
    if (i == 4) rm(area.sq)
    r[i] <- h()
  }
  r
}
g()
# a method passed as an argument: a promise in the calling frame
k <- function(area.sq) area(x)
k(function(s) "sq argument")
k()
# the defining environment's S3 methods table
.__S3MethodsTable__. <- new.env()
r <- character(0)
for (i in 1:5) {
  if (i == 2) assign("area.sq", function(s) "sq table", envir = .__S3MethodsTable__.)
  if (i == 3) area.sq <- function(s) "sq global before the table"
  if (i == 4) rm(area.sq)
  if (i == 5) rm("area.sq", envir = .__S3MethodsTable__.)
  r[i] <- area(x)
}
r
rm(.__S3MethodsTable__.)
# a frame binding .packageName as the top environment: global comes after it
mk <- function() { .packageName <- "pkg"; function(o) area(o) }
ca <- mk()
r <- character(0)
for (i in 1:5) {
  if (i == 2) area.shape <- function(s) "shape global, after the top"
  if (i == 3) assign("area.sq", function(s) "sq in the top frame", envir = environment(ca))
  if (i == 4) rm("area.sq", envir = environment(ca))
  if (i == 5) rm(area.shape)
  r[i] <- ca(x)
}
r
# the generic's environment: a new defining environment and its table
e2 <- new.env()
assign(".packageName", "p2", envir = e2)
assign(".__S3MethodsTable__.", new.env(), envir = e2)
assign("area.sq", function(s) "sq in e2's table", envir = get(".__S3MethodsTable__.", envir = e2))
area2 <- function(s) UseMethod("area")
r <- character(0)
for (i in 1:3) {
  if (i == 2) environment(area2) <- e2
  if (i == 3) environment(area2) <- globalenv()
  r[i] <- area2(x)
}
r
# the class vector
r <- character(0)
for (i in 1:3) {
  if (i == 2) class(x) <- "shape"
  if (i == 3) class(x) <- c("tri", "sq")
  r[i] <- area(x)
}
r
area.shape <- function(s) "shape again"
area(x)
class(x) <- "shape"
area(x)
# a method bound into the calling frame by assign(), outside its shape
x <- structure(1, class = c("sq", "shape"))
f2 <- function() {
  r <- character(0)
  for (i in 1:3) {
    if (i == 2) assign("area.sq", function(s) "sq by assign", envir = environment())
    if (i == 3) rm("area.sq", envir = environment())
    r[i] <- area(x)
  }
  r
}
f2()
# a frame below the caller becomes the top environment (.packageName):
# the methods table then comes before the global environment
.__S3MethodsTable__. <- new.env()
assign("area.sq", function(s) "sq table", envir = .__S3MethodsTable__.)
area.sq <- function(s) "sq global"
f4 <- function() {
  h <- function() area(x)
  r <- character(0)
  for (i in 1:3) {
    if (i == 2) assign(".packageName", "p4", envir = environment())
    if (i == 3) rm(".packageName", envir = environment())
    r[i] <- h()
  }
  r
}
f4()
rm(area.sq)
rm(.__S3MethodsTable__.)
# a method that changes its .Generic in place changes its own copy
gg <- function(x) UseMethod("gg")
gg.a <- function(x) { .Generic[1] <- "changed"; c(.Generic, .Method) }
gg.b <- function(x) c(.Generic, .Method)
for (i in 1:2) print(gg(structure(1, class = "a")))
gg(structure(1, class = "b"))
