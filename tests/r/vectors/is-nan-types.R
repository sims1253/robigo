# is.nan() is FALSE for every element of a character, raw, logical or
# integer vector (coerce.c:do_isnan), keeping dim and names; other types
# are an error.
is.nan(as.raw(c(0, 1, 255)))
is.nan(c(a = "x", b = NA, c = "NaN"))
is.nan(character())
is.nan(raw())
is.nan(matrix(as.raw(1:4), 2))
is.nan(c(TRUE, NA))
is.nan(c(1L, NA))
is.nan(NULL)
is.nan(c(1, NA, NaN, Inf))
is.nan(complex(real = NaN, imaginary = 1))
tryCatch(is.nan(list(1)), error = conditionMessage)
tryCatch(is.nan(quote(x)), error = conditionMessage)
