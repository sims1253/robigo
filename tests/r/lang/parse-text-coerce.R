# R: source.c:do_parse — the text argument is coerced to character
# first (coerceVector(CAR(args), STRSXP)); an expression or call deparses
# into text. The error fires only when the argument had length but the
# coercion produced none. jsonlite's serializeJSON round trips S4
# expression objects through parse(text = <expression>).
e <- parse(text = "foo; bar", keep.source = FALSE)
print(e)
print(parse(text = e))
print(parse(text = expression(foo)))
print(parse(text = quote(a + b)))
print(parse(text = 1:3))
print(parse(text = character(0)))
print(try(parse(text = list(a = 1)), silent = TRUE))
# no text at all: the file argument (here a text connection)
f <- textConnection("x <- 1\ny <- 2\n")
print(parse(f))
close(f)
