# base's S3 methods table (registered by baseloader.R) and the method lookup
# memo: bindings made in the table between dispatches of `$` on an object
# whose methods are found nowhere, in the table, or in base after it.
tbl <- baseenv()[[".__S3MethodsTable__."]]
cat(class(tbl), length(ls(tbl, all.names = TRUE)) > 300, "\n")
cat(identical(tbl$print.default, print.default), identical(tbl[["unique.default"]], unique.default), "\n")
x <- structure(list(side = 2), class = "sq")
f <- function(s) s$side
show <- function() for (i in 1:3) print(f(x))
show()
# a non-function in the table: no method, the default `$` as before
assign("$.sq", 42, envir = tbl)
show()
# a function in the table: the method
assign("$.sq", function(x, name) "from the table", envir = tbl)
show()
assign("$.sq", 43, envir = tbl)
show()
rm("$.sq", envir = tbl)
show()
# a method found after the table: in the enclosure of a top environment
# that binds .packageName
outer <- new.env()
pkg <- new.env(parent = outer)
assign(".packageName", "fake", envir = pkg)
fp <- local(function(s) s$side, envir = pkg)
showp <- function() for (i in 1:3) print(fp(x))
showp()
assign("$.sq", function(x, name) "from outer", envir = outer)
showp()
# hidden by a non-function in the table
assign("$.sq", "not a function", envir = tbl)
showp()
rm("$.sq", envir = tbl)
showp()
# a promise in the table, forced by the first lookup
delayedAssign("$.sq", { cat("forcing\n"); function(x, name) "promised" }, assign.env = tbl)
showp()
show()
rm("$.sq", envir = tbl)
showp()
rm("$.sq", envir = outer)
showp()
show()
# methods of base generics registered in the table only: an internal
# generic and a closure's UseMethod
assign("length.sq", function(x) 99L, envir = tbl)
assign("unique.sq", function(x, incomparables = FALSE, ...) "sq unique", envir = tbl)
for (i in 1:2) print(c(length(x), length(unclass(x))))
for (i in 1:2) print(unique(x))
rm("length.sq", "unique.sq", envir = tbl)
for (i in 1:2) print(length(x))
for (i in 1:2) print(unique(x))
