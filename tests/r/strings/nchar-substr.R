# nchar, nzchar, substr, substr<-, startsWith, endsWith, strtrim, strrep
nchar(c("abc", "", NA, "hello world"))
nchar(c("abc", NA), type = "bytes")
nchar(c("abc", NA), type = "width")
nchar(NA)
nchar(NA_character_)
nchar(c(NA, "x"), keepNA = FALSE)
nchar(c(one = "a", two = "bb"))
nchar(123456)
nchar(c(1.5, 10, NA))
nchar(TRUE)
nchar(NULL)
nchar(character(0))
nchar("abc", type = "c")
nchar("abc", "b")
nzchar(c("a", "", NA))
nzchar(c("a", "", NA), keepNA = TRUE)
nzchar(NULL)
nzchar(1:3)

substr("abcdef", 2, 4)
substr(c("abc", NA, "defgh"), 2, 3)
substr("abc", 0, 1)
substr("abc", 3, 2)
substr("abc", NA, 2)
substr("abc", 2, 100)
substr(c(a = "xyz", b = "uvw"), 1, 2)
substr(123456, 2, 4)
substr("abcdef", 1:3, 3:5)
substr(character(0), 1, 2)

x <- "abcdef"
substr(x, 2, 3) <- "ZZZZ"
x
x <- "abcdef"
substr(x, 5, 10) <- "ZZZZ"
x
x <- c("abc", "defg")
substr(x, 2, 3) <- c("XY", "Z")
x
x <- "abc"
substr(x, 1, 1) <- ""
x
x <- "ab"
substr(x, 1, 1) <- NA_character_
x
x <- c(a = "hello")
substr(x, 2, 3) <- "EY"
x

startsWith("foobar", "foo")
endsWith("foobar", "bar")
startsWith(c("apple", "banana", NA), "a")
startsWith(c("apple", "banana", "cherry"), c("a", "b"))
endsWith("foobar", c("bar", "foo", ""))
endsWith(character(0), "a")
startsWith("abc", character(0))

strtrim("abcdef", c(1, 3))
strtrim(c("abcdef", "ab", NA), c(3, 1, 2))
strtrim(c(a = "xyz"), 2)
strtrim("abc", 0)

strrep("ab", 3)
strrep(c("a", "b"), 1:2)
strrep("x", c(0, 1, 2, NA))
strrep(NA, 2)
strrep(c(x = "a"), 2)
strrep(c(x = "a"), 1:2)
strrep(character(0), 2)
