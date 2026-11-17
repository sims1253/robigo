# S3 method lookup memo (g3.md §9.3) and the defining environment's S3
# methods table: base's (a lazy binding; `length`'s internal dispatch) and
# one the global environment binds (UseMethod), methods registered,
# replaced by something that is not a function, and removed between
# dispatches, with a method after the table.
x <- structure(list(1, 2), class = "sq")
tbl <- get(".__S3MethodsTable__.", envir = baseenv())
f <- function() length(x)
f()
assign("length.sq", function(x) 99L, envir = tbl)
f()
assign("length.sq", "not a function", envir = tbl)
f()
rm("length.sq", envir = tbl)
f()
length.sq <- function(x) -1L
f()
assign("length.sq", function(x) 7L, envir = tbl)
f()
rm(length.sq)
f()
rm("length.sq", envir = tbl)
f()
# a top environment that is not the global one (a frame binding
# .packageName): the global environment comes after the table, and its
# method is found until the table binds the name
mk <- function() { .packageName <- "pkg"; list(len = function() length(x), area = function() area(x)) }
m <- mk()
p <- m$len
length.sq <- function(x) 42L
p()
assign("length.sq", 1, envir = tbl)
p()
rm("length.sq", envir = tbl)
p()
rm(length.sq)
p()
# the global environment's own table, for a generic defined there
area <- function(s) UseMethod("area")
area.default <- function(s) "default"
area.sq <- function(s) "sq global"
k <- m$area
k()
.__S3MethodsTable__. <- new.env()
k()
assign("area.sq", "not a function", envir = .__S3MethodsTable__.)
k()
assign("area.sq", function(s) "sq in the table", envir = .__S3MethodsTable__.)
k()
rm("area.sq", envir = .__S3MethodsTable__.)
k()
.__S3MethodsTable__. <- 5
k()
rm(.__S3MethodsTable__.)
k()
rm(area.sq)
k()
