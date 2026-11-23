# R: printarray.c:_COMPUTE_W2_ / printutils.c:Rstrwid — a dimname's column
# width is the *escaped* display width (Rstrwid counts "\\" as two, and
# EncodeString prints it doubled), so a backslash in a column name widens
# the column by one.
m <- matrix(c(TRUE, FALSE), 1, 5, dimnames = list(NULL, c("#ifdef", "\\Sexpr", "build", "install", "render")))
print(m)

# Row labels escape the same way (Rstrlen), and the widest escaped label
# sets the row-label field.
print(matrix(1:4, 2, 2, dimnames = list(c("a\\b", "c"), c("x", "y"))))

# A quote in a label is not escaped (quote = 0 in the matrix label path),
# but the backslash still is.
print(matrix(1:4, 1, 4, dimnames = list(NULL, c("a\"b", "c\\d", "e", "f"))))

# Numeric and string matrices take the same label path.
print(matrix(c(1.5, 2.5), 1, 2, dimnames = list(NULL, c("p\\q", "r"))))
print(matrix(c("u", "v"), 1, 2, dimnames = list(NULL, c("p\\q", "r"))))
