# PrintGenericVector: each element under its tag ($name, $`odd name`,
# [[i]]), nested tags, NULL elements, empty lists, NA strings vs "NA".
list(1, "a", TRUE, NULL, 1:3)
list(a = 1, b = "x", c = list(d = 2, e = list(f = NULL)))
list(a = 1, 2, `b c` = 3, `NA` = 4, `if` = 5)
list()
list(list(), list(list()))
list(list(1, list(2, list(3))))
list(NA_character_, "NA", NA, NA_integer_, NA_real_)
list(x = list(1:30))
list(1i, 1:3 + 2i)
