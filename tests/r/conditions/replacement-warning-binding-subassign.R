# Subassignment and attribute setters: handlers and methods they run see
# the target's current value (#527).
h <- function(w) { print(x); invokeRestart("muffleWarning") }
x <- 1:3
withCallingHandlers(x[1:2] <- 1:3, warning = h)
x <- list(1, 2, 3)
withCallingHandlers(x[1:2] <- list(1, 2, 3), warning = h)
x <- 1:3
withCallingHandlers(x[[2]] <- 1:2, warning = h, error = function(e) print(x))
x <- c(a = 1)
withCallingHandlers(levels(x) <- "a", warning = h)
l <- list(a = "bad")
withCallingHandlers(class(l$a) <- "integer", warning = function(w) { print(l); invokeRestart("muffleWarning") })
print(l)
x <- 1:2
as.character.zz <- function(x, ...) { cat("method sees: "); print(get("x", parent.frame(2))); c("p", "q") }
names(x) <- structure(1:2, class = "zz")
print(x)
x <- "abc"
withCallingHandlers(substr(x, 1, 1) <- "z", warning = h)
x <- 1:3
withCallingHandlers(length(x) <- 2.5, warning = h)
