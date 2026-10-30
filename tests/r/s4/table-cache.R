library(methods)
setClass("CacheBase", slots = c(x = "numeric", optional = "ANY"))
setClass("CacheChild", contains = "CacheBase")
setGeneric("cachedValue", function(x) standardGeneric("cachedValue"))
setMethod("cachedValue", "CacheBase", function(x) x@x)
x <- new("CacheChild", x = 1)
print(cachedValue(x))
print(cachedValue(x))
setMethod("cachedValue", "CacheBase", function(x) x@x + 10)
print(cachedValue(x))
setMethod("cachedValue", "CacheChild", function(x) x@x + 100)
print(cachedValue(x))
removeMethod("cachedValue", "CacheChild")
print(cachedValue(x))
# A method can redefine itself during a dispatch.
setMethod("cachedValue", "CacheChild", function(x) {
    setMethod("cachedValue", "CacheChild", function(x) x@x + 1000)
    x@x + 200
})
print(cachedValue(x))
print(cachedValue(x))
# R_set_slot updates initialize's object binding, including NULL slots.
setMethod("initialize", "CacheChild", function(.Object, ...) {
    slot(.Object, "x", check = FALSE) <- 7
    slot(.Object, "optional", check = FALSE) <- NULL
    .Object
})
y <- new("CacheChild")
print(y@x)
print(y@optional)
z <- y
z@x <- 9
print(c(y@x, z@x))
# The live table can contain an active binding; never cache its result.
tab <- get(".AllMTable", envir = environment(getGeneric("cachedValue")))
definition <- selectMethod("cachedValue", "CacheChild")
rm(list = "CacheChild", envir = tab)
count <- 0L
makeActiveBinding("CacheChild", function() {
    count <<- count + 1L
    definition
}, tab)
print(cachedValue(y))
print(cachedValue(y))
print(count)
