# A replacement call with the wrong number of arguments fails its
# primitive's opening checkArity (#548 runs it before the body) and
# leaves the target as it was, on the in-place paths too (#527).
x <- c(a = 1)
r <- try(attr(x) <- 2, silent = TRUE)
cat(r)
print(x)
r <- try(names(x, 1) <- "b", silent = TRUE)
cat(r)
print(x)
r <- try(class(x, 1) <- "foo", silent = TRUE)
cat(r)
print(x)
r <- try(levels(x, 2) <- "q", silent = TRUE)
cat(r)
print(x)
l <- list(a = list(b = 1))
r <- try(attr(l$a) <- 2, silent = TRUE)
cat(r)
print(l)
f <- function() {
  v <- 1:3
  r <- try(dim(v, 1) <- 3L, silent = TRUE)
  cat(r)
  v
}
print(f())
e <- new.env()
r <- try(environment(e, 1) <- e, silent = TRUE)
cat(r)
print(class(e))
