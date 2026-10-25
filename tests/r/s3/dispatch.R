print.dog <- function(x, ...) {
  cat("Dog named", x$name, "\n")
  invisible(x)
}
d <- structure(list(name = "Rex"), class = "dog")
d
print(d)
speak <- function(x, ...) UseMethod("speak")
speak.default <- function(x, ...) cat("...\n")
speak.dog <- function(x, ...) {
  cat("Woof\n")
  NextMethod()
}
speak.animal <- function(x, ...) cat("generic animal noise\n")
a <- structure(list(), class = c("dog", "animal"))
speak(a)
speak(1)
inherits(a, "animal")
inherits(a, c("cat", "animal"), which = TRUE)
class(a)
unclass(d)
format.money <- function(x, ...) paste0("$", formatC(unclass(x), format = "f", digits = 2))
print.money <- function(x, ...) print(format(x), quote = FALSE)
m <- structure(c(1.5, 20), class = "money")
m
toString.dog <- function(x, ...) paste("dog:", x$name)
toString(d)
summary.dog <- function(object, ...) "a summary"
summary(d)
