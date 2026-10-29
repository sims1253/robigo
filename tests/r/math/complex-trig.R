# The complex Math group (R 4.5 complex.c): glibc kernels plus R's
# branch-cut wrappers (z_asin/z_acos/z_atan and the -I * f(z * I)
# compositions for the hyperbolic inverses).  NA in gives complex NA;
# a NaN produced warns "NaNs produced in function \"<name>\"".
z <- c(0+0i, 1+0i, -1+0i, 0+1i, 0-1i, 1+1i, 2-0.5i, -3+4i, 0.5+0.5i,
       100+100i, 1e5+0.5i, 0.1-200i, 2+26i, complex(real = NA, imaginary = 1),
       complex(real = 1, imaginary = NA), complex(real = NaN, imaginary = 1),
       complex(real = Inf, imaginary = 1), complex(real = -Inf, imaginary = 0),
       complex(real = 0, imaginary = -Inf), 1e-300+1e-300i, 3+0i, 0.999+0i,
       1.001+0i, complex(real = 0, imaginary = 1.001),
       complex(real = 0, imaginary = 0.999), 5+0i, -5+0i)
fns <- list(cos, sin, tan, acos, asin, atan, cosh, sinh, tanh, acosh, asinh, atanh)
nms <- c("cos", "sin", "tan", "acos", "asin", "atan", "cosh", "sinh", "tanh",
         "acosh", "asinh", "atanh")
for (i in seq_along(fns)) {
    r <- tryCatch(fns[[i]](z), warning = function(w) {
        cat("warn:", conditionMessage(w), "\n"); suppressWarnings(fns[[i]](z))
    })
    cat(nms[i], ":", format(r, digits = 15), "\n")
}
# the log family with a base goes through complex_math2 (z_logbase)
cat("log10:", format(log10(z), digits = 15), "\n")
cat("log2:", format(log2(z), digits = 15), "\n")
cat("logb:", format(log(z, 3), digits = 15), "\n")
# unimplemented ones error with the call reported
for (f in c("floor", "ceiling", "sign", "expm1", "log1p", "gamma", "lgamma",
            "digamma", "trigamma", "sinpi", "cospi", "tanpi"))
    tryCatch(eval(parse(text = paste0(f, "(1+1i)"))),
             error = function(e) cat("err:", conditionMessage(e), "\n"))
# a names attribute survives
z2 <- structure(c(1+1i, 2+2i), names = c("a", "b"))
print(tan(z2))
