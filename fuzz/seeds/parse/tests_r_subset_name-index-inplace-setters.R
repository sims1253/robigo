# Character subscripts look names up through a cached index (#546). The
# in-place replacement paths (#527) must leave no stale index behind:
# every lookup after a setter sees the names as they now are.
nm <- paste0("n", 1:40)

x <- 1:40
names(x) <- nm
x[["n5"]]; x[["n5"]]; x[c("n1", "n40")]
names(x)[5] <- "five"
print(x[["five"]]); print(try(x[["n5"]], silent = TRUE) |> inherits("try-error"))
attr(x, "names") <- rev(nm)
print(x[["n1"]]); print(x[c("n40", "n2")])
attributes(x) <- list(names = paste0("m", 1:40))
print(x[["m7"]]); print(is.na(x["n7"]))
x[["new"]] <- 99L
print(x[["new"]]); print(length(x))
names(x) <- NULL
print(is.null(names(x)))
names(x) <- c(nm, "z")
print(x[["n33"]])
storage.mode(x) <- "double"
print(x[["n33"]])
class(x) <- "foo"
print(unclass(x)[["n12"]])
l <- as.list(1:40); names(l) <- nm
l$n3; l[["n3"]]
names(l)[3] <- "three"
print(l[["three"]]); print(is.null(l[["n3"]]))
l$z <- "zz"; print(l[["z"]])
f <- function() {
  y <- 1:40; names(y) <- nm
  y[["n9"]]; y[["n9"]]
  names(y)[9] <- "nine"
  attr(y, "names")[10] <- "ten"
  c(y[["nine"]], y[["ten"]])
}
print(f())
