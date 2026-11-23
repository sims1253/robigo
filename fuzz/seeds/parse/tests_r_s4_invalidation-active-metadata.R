library(methods)
setGeneric("activeMetadata", function(x) standardGeneric("activeMetadata"))
setMethod("activeMetadata", "numeric", function(x) x + 1)
fenv <- environment(getGeneric("activeMetadata"))
print(activeMetadata(1))
# GNU R reads each metadata binding once per dispatch, before forcing x.
for (name in c(".AllMTable", ".SigArgs", ".SigLength")) {
    local({
        key <- name
        value <- get(key, fenv)
        rm(list = key, envir = fenv)
        makeActiveBinding(key, function(v) {
            if (!missing(v)) stop("unexpected write")
            cat(key, "\n")
            value
        }, fenv)
    })
}
print(activeMetadata({cat("force x\n"); 2}))
print(activeMetadata(3))
