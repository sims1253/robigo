print.thing <- function(x, ...) {
  cat("<thing", unclass(x), ">\n")
  invisible(x)
}
t1 <- structure(42, class = "thing")
t1
print(t1)
z <- print(t1)
toString.thing <- function(x, ...) "thing!"
toString <- function(x, ...) UseMethod("toString")
toString(t1)
