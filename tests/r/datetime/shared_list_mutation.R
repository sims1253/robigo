## GNU R's valid_POSIXlt assigns the coerced components back into its
## argument with SET_VECTOR_ELT (datetime.c), so a VECSXP is changed through
## every alias: a second binding, an environment field and an attribute all
## see the new cells. The R-level replacement primitives are the operations
## that copy a shared object first.
mk <- function() {
  x <- list(1L, 0L, 0L, 1L, 0L, 70L, 0L, 0L, 0L)
  names(x) <- c("sec", "min", "hour", "mday", "mon", "year", "wday", "yday", "isdst")
  class(x) <- c("POSIXlt", "POSIXt")
  x
}

## Two bindings to the same list.
x <- mk()
y <- x
invisible(balancePOSIXlt(x))
cat("bindings:", typeof(unclass(x)[[1]]), typeof(unclass(y)[[1]]), "\n")
cat("returned:", typeof(unclass(balancePOSIXlt(mk()))[[1]]), "\n")

## A list inside an environment.
e <- new.env()
e$x <- mk()
e$y <- e$x
invisible(balancePOSIXlt(e$x))
cat("environment:", typeof(unclass(e$x)[[1]]), typeof(unclass(e$y)[[1]]), "\n")

## A list inside an attribute.
o <- structure(list(), tag = mk())
held <- attr(o, "tag")
invisible(balancePOSIXlt(attr(o, "tag")))
cat("attribute:", typeof(unclass(attr(o, "tag"))[[1]]), typeof(unclass(held)[[1]]), "\n")

## The callers that duplicate first leave their argument alone.
z <- mk()
invisible(as.POSIXct(z, "UTC"))
cat("as.POSIXct:", typeof(unclass(z)[[1]]), "\n")
z <- mk()
invisible(format(z, "%Y"))
cat("format:", typeof(unclass(z)[[1]]), "\n")
z <- mk()
invisible(.Internal(POSIXlt2Date(z)))
cat("POSIXlt2Date:", typeof(unclass(z)[[1]]), "\n")

## An error after the first store keeps the stores already made, as in GNU R.
bad <- mk()
names(bad) <- c(names(bad), "zone")
bad[[10]] <- 1L
invisible(tryCatch(balancePOSIXlt(bad), error = function(e) cat("error:", conditionMessage(e), "\n")))
cat("partial:", typeof(unclass(bad)[[1]]), typeof(unclass(bad)[[2]]), "\n")

## R-level replacement copies a shared list instead.
a <- list(1L)
b <- a
a[[1]] <- 2L
cat("replacement:", b[[1]], a[[1]], "\n")
