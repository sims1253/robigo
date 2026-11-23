# replacement methods ([[<-, [<-, items<-), $ and [[ methods, dispatch on ..., generics from existing functions
library(methods)
setClass("Bag", representation(items = "list"))
setMethod("[[", "Bag", function(x, i, ...) x@items[[i]])
setMethod("$", "Bag", function(x, name) x@items[[name]])
setReplaceMethod("[[", "Bag", function(x, i, ..., value) { x@items[[i]] <- value; x })
setMethod("[<-", "Bag", function(x, i, ..., value) { x@items[i] <- value; x })
setMethod("names", "Bag", function(x) names(x@items))
setGeneric("items<-", function(x, value) standardGeneric("items<-"))
setReplaceMethod("items", "Bag", function(x, value) { x@items <- value; x })
b <- new("Bag", items = list(a = 1, b = "two"))
b[["a"]]
b$b
b[["c"]] <- 3
names(b)
b["a"] <- list(10)
b$a
items(b) <- list(z = 0)
names(b)
setGeneric("total", function(...) standardGeneric("total"), signature = "...")
setMethod("total", "numeric", function(...) sum(...))
total(1, 2, 3)
r <- tryCatch(total("a", "b"), error = function(e) conditionMessage(e)); cat(r, "\n")
area <- function(shape, ...) "default area"
setGeneric("area")
setClass("Sq", representation(s = "numeric"))
setMethod("area", "Sq", function(shape, ...) shape@s^2)
area(1)
area(new("Sq", s = 4))
showMethods("area")
setGeneric("scale2", function(x, k = 2) standardGeneric("scale2"))
setMethod("scale2", "numeric", function(x, k = 2) x * k)
scale2(3); scale2(3, k = 10); scale2(k = 5, x = 1)
