library(methods)
setClass("GuardPoint", slots = c(x = "numeric", nullable = "NULL"))
x <- new("GuardPoint", x = 7)
read <- function(x) x@x
for (i in seq_len(100)) stopifnot(identical(read(x), 7))
print(read(x))
print(x@nullable)
y <- x
y@x <- 8
print(c(read(x), read(y)))
# GNU R compiles the warmed read before this global @ replacement.
`@` <- function(object, name) 100
print(read(x))
rm(`@`)
print(read(x))
readLocal <- local({
    `@` <- function(object, name) 200
    function(x) x@x
})
print(readLocal(x))
# S3 dispatch and missing slots need the ordinary primitive path.
`@.guardS3` <- function(object, name) 300
print(read(structure(list(), class = "guardS3")))
print(tryCatch(read(1), error = function(e) conditionMessage(e)))
print(tryCatch(x@absent, error = function(e) conditionMessage(e)))
