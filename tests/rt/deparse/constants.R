# Constants in calls (vector2buff with keepInteger, keepNA): integers with
# L, NA_integer_, doubles at 15 significant digits, strings escaped.
quote(f(1L, NA, 3.5, "s", TRUE, NULL))
quote(f(1e-20, 1e+20, 123456789012, 0.1, 1/3))
quote(f(Inf, -Inf, NaN, NA_real_, NA_integer_, NA_character_))
quote("string with \"quotes\" and \n newline")
quote(f("tab\t", "é", "日本"))
quote(-1L)
quote(f(100000, 1e5 + 0.1, 0.1 + 0.2))
