dyn.load("fortran_strings.so")

str(.Fortran("cfort", "hello", n = 0L))
str(.Fortran("cfort", c("abc", "defgh"), n = 0L))
str(.Fortran("cfort", NA_character_, n = 0L))
r <- .Fortran("cfortlong", strrep("a", 10))
print(nchar(r[[1]]))
