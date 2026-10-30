library(methods)
setClass("PerfHolding", slots = c(price = "numeric", units = "numeric"),
         validity = function(object) {
             if (any(object@units < 0)) "negative units" else TRUE
         })
setClass("PerfEquity", contains = "PerfHolding")
setGeneric("marketValue", function(object) standardGeneric("marketValue"))
setMethod("marketValue", "PerfHolding", function(object) sum(object@price * object@units))
# Mark a portfolio, check its constraints, and value it through an inherited method.
execute <- function(n) {
    holding <- new("PerfEquity", price = c(10, 20, 30), units = c(3, 2, 1))
    total <- 0
    for (i in seq_len(n)) {
        holding@price <- holding@price + 1
        validObject(holding)
        total <- total + marketValue(holding)
    }
    total
}
stopifnot(identical(execute(100L), 40300))
