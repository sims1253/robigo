library(methods)
setClass("ScopeValue", slots = c(x = "numeric"))
setGeneric("scopeValue", function(x) standardGeneric("scopeValue"))
setMethod("scopeValue", "ScopeValue", function(x) x@x)
x <- new("ScopeValue", x = 5)
callValue <- function(x) scopeValue(x)
print(callValue(x))
print(local({
    scopeValue <- function(x) -10
    c(scopeValue(x), callValue(x))
}))
saved <- scopeValue
scopeValue <- function(x) -20
print(c(callValue(x), saved(x)))
scopeValue <- saved
print(callValue(x))
# Same generic name in a distinct lexical environment and method table.
e <- new.env()
evalq({
    setGeneric("scopeValue", function(x) standardGeneric("scopeValue"), where = environment())
    setMethod("scopeValue", "ScopeValue", function(x) x@x + 100, where = environment())
}, e)
print(c(e$scopeValue(x), scopeValue(x), e$scopeValue(x)))
