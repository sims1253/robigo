# S3 dispatch translates a class name (translateChar): a latin1 class finds
# its methods by their native (UTF-8) names
t <- function(expr) tryCatch(expr, error = function(e) print(e))
k <- "caf\xe9"; Encoding(k) <- "latin1"
x <- structure(1, class = c(k, "base"))
f <- function(x) UseMethod("f")
assign(paste0("f.", k), function(x) c("latin1 method", NextMethod()))
f.base <- function(x) "base method"
print(f(x))
g <- function(x) UseMethod("g")
registerS3method("g", k, function(x) "registered method")
print(g(x))
assign(paste0("Ops.", k), function(e1, e2) paste("Ops for", .Generic))
print(x + 1)
# a generic name that is latin1 too
gn <- "g\xe9n"; Encoding(gn) <- "latin1"
assign(gn, function(x) UseMethod(gn))
assign(paste0(gn, ".", k), function(x) "latin1 generic")
print(get(gn)(x))
# no method: the message names the class
t(print(f(structure(1, class = k))))
h <- function(x) UseMethod("h")
t(h(structure(1, class = k)))
# a "bytes" class name cannot be translated
b <- "caf\xe9"; Encoding(b) <- "bytes"
t(f(structure(1, class = b)))
# NextMethod falls back to the generic's primitive (getPrimitive): a closure
# masking a builtin, its base binding
length <- function(x) UseMethod("length")
length.zz <- function(x) NextMethod()
print(length(structure(1:3, class = "zz")))
rm(length)
