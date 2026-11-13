# Complex numbers as strings (StringFromComplex, 15 digits), through
# paste and string comparison; cat writes them in 7 digits.
paste(1+2i, 1/3 + 1i, -1.5 - 0.25i, NA_complex_, NaN + 1i)
paste0(1i * (1:3))
(1+2i) == "1+2i"
cat(1/3 + 1i, 1e10 + 1i, "\n")
