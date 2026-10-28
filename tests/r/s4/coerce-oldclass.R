# setAs, setOldClass, S4 methods for S3 classes, initialize + callNextMethod, Arith methods, valueClass, show in lists
library(methods)
setClass("Celsius", contains = "numeric")
setClass("Fahrenheit", contains = "numeric")
setAs("Celsius", "Fahrenheit", function(from) new("Fahrenheit", from@.Data * 9/5 + 32))
as(new("Celsius", 100), "Fahrenheit")
setOldClass("myS3")
setGeneric("describe", function(x) standardGeneric("describe"))
setMethod("describe", "myS3", function(x) "an S3 object")
setMethod("describe", "data.frame", function(x) paste("df with", nrow(x), "rows"))
describe(structure(list(), class = "myS3"))
describe(data.frame(a = 1:3))
setClass("Base", representation(id = "numeric"))
setMethod("initialize", "Base", function(.Object, ...) {
  .Object <- callNextMethod(.Object, ...)
  if (length(.Object@id) == 0) .Object@id <- 99
  .Object
})
new("Base")@id
new("Base", id = 5)@id
setClass("Money", representation(amount = "numeric", cur = "character"))
setMethod("Arith", signature("Money", "Money"), function(e1, e2) {
  new("Money", amount = callGeneric(e1@amount, e2@amount), cur = e1@cur)
})
setMethod("show", "Money", function(object) cat(object@amount, object@cur, "\n"))
new("Money", amount = 5, cur = "EUR") + new("Money", amount = 2, cur = "EUR")
new("Money", amount = 5, cur = "EUR") * new("Money", amount = 2, cur = "EUR")
setGeneric("toS", function(x, ...) standardGeneric("toS"), valueClass = "character")
setMethod("toS", "ANY", function(x, ...) as.character(x))
toS(1:3)
is(new("Money", amount = 1, cur = "X"))
setMethod("summary", "Money", function(object, ...) cat("Money summary\n"))
summary(new("Money", amount = 1, cur = "X"))
setMethod("length", "Money", function(x) 1L)
length(new("Money", amount = 1, cur = "X"))
x <- list(new("Money", amount = 3, cur = "USD"), 2)
x
format(new("Celsius", 1.5))
