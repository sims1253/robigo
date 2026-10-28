# R: array.c:CMATPROD_BODY and the two cross-product variants.
# Keep NA/NaN payload distinctions visible without complex deparsing.
v <- c(NA_real_, NaN, 0, 1, Inf)
x <- list()
for (re in v) for (im in v)
    x[[length(x) + 1L]] <- complex(real = re, imaginary = im)
code <- function(x) if (is.nan(x)) "NaN" else if (is.na(x)) "NA" else as.character(x)
for (op in 0:2) {
    out <- character()
    for (a in x) for (b in x) {
        z <- switch(op + 1L, a %*% b, crossprod(a, b), tcrossprod(a, b))
        out <- c(out, paste(code(Re(z)), code(Im(z)), sep = "/"))
    }
    cat(paste(out, collapse = " "), "\n")
}
as.complex(NA) %*% NaN
# Accumulation with competing payloads.
for (a in x) for (b in x) {
    z <- matrix(c(a, b), 1L) %*% c(1+0i, 1+0i)
    cat(code(Re(z)), code(Im(z)), "\n")
}
# Finite inputs can still overflow or underflow inside the plain product.
x <- complex(real = c(1e308, -1e308, 1e-308, 0),
             imaginary = c(1e308, 1e308, -1e-308, 1e-308))
for (a in x) for (b in x) {
    for (z in list(a %*% b, crossprod(a, b), tcrossprod(a, b)))
        cat(code(Re(z)), code(Im(z)), "\n")
}
