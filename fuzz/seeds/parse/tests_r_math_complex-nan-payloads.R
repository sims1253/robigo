# Which NaN survives complex arithmetic: GNU R prints NA or NaN by the
# payload, so the operand order of its compiled + * %*% shows.
bits <- function(z) {
  b <- writeBin(z, raw())
  paste(paste(rev(as.character(b[1:8])), collapse = ""), paste(rev(as.character(b[9:16])), collapse = ""))
}
na <- 1i + NA          # (NA, 1)
nan <- as.complex(0/0) # (NaN, 0)
pn <- -(0/0) + 0i
print(c(na * nan, nan * na, na + nan, nan + na, na - nan, nan - na, na / nan, nan / na))
print(c(bits(na * nan), bits(nan * na), bits(na + nan), bits(nan + na)))
print(c(bits(pn * na), bits(na * pn), bits(complex(real = NA, imaginary = 0/0) * (1 + 2i))))
print(c(NA) %*% c(as.complex(NaN)))
print(c(as.complex(NA)) %*% c(NaN))
print(matrix(c(na, nan), 1) %*% matrix(c(nan, na), 2))
print(crossprod(c(na, 1i), c(nan, 2)))
print(cumprod(c(na, nan, 2)))
print(prod(c(nan, na)))
print((0/0) * (1i + NA))
print((1i + NA) * (-(0/0)))
print(((0/0) + 1i) * (3.4 + NA))
print(c(na, nan)^2)
