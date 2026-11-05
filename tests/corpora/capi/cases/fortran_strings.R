dyn.load("fortran_strings.so")

print(.Fortran("cfort", "hello", n = 0L))
print(.Fortran("cfort", c("abc", "defgh"), n = 0L))
print(.Fortran("cfort", NA_character_, n = 0L))
r <- .Fortran("cfortlong", strrep("a", 10))
print(nchar(r[[1]]))
