# chartr (character.c do_chartr)
chartr("abc", "xyz", c("aabbcc", "cab", NA, ""))
chartr("a-c", "A-C", "abcdef")
chartr("a-cx", "D-Fy", "abcxyz")
# a character named twice maps as its last place says
chartr("aa", "xy", "banana")
chartr("abc", "defg", "abc")
x <- c(first = "hello", second = "world")
chartr("lo", "01", x)
f <- factor("a"); chartr("a", "b", f)
m <- matrix(c("ab", "ba", "aa", "bb"), 2); chartr("ab", "ba", m)
# UTF-8
chartr("éè", "ee", "étè")
Encoding(chartr("é", "e", "café"))
chartr("a", "é", "banana")
Encoding(chartr("a", "é", "banana"))
chartr("α-γ", "a-c", "αβγδ")
# latin1 input comes back UTF-8
l <- "caf\xe9"; Encoding(l) <- "latin1"
r <- chartr("f", "F", l); r; Encoding(r)
chartr(l, "abcd", "face")
# numbers are coerced by the wrapper
chartr("1", "9", 1:12)
# errors and warnings
tryCatch(chartr("abc", "ab", "x"), error = function(e) print(conditionMessage(e)))
tryCatch(chartr("c-a", "abc", "x"), error = function(e) print(conditionMessage(e)))
tryCatch(chartr("γ-α", "abc", "x"), error = function(e) print(conditionMessage(e)))
tryCatch(chartr(NA, "a", "x"), error = function(e) print(conditionMessage(e)))
tryCatch(chartr(character(0), "a", "x"), error = function(e) print(conditionMessage(e)))
tryCatch(chartr(1, "a", "x"), error = function(e) print(conditionMessage(e)))
tryCatch(chartr("a", NA_character_, "x"), error = function(e) print(conditionMessage(e)))
tryCatch(.Internal(chartr("a", "b", 1)), error = function(e) print(conditionMessage(e)))
withCallingHandlers(chartr(c("a", "b"), c("x", "y"), "ab"),
                    warning = function(w) { print(conditionMessage(w)); invokeRestart("muffleWarning") })
g <- function(o) chartr(o, "x", "y")
tryCatch(g("ba-a"), error = function(e) print(conditionCall(e)))
b <- "a\xff"; Encoding(b) <- "bytes"
tryCatch(chartr("a", "b", b), error = function(e) print(conditionMessage(e)))
bad <- "a\xff"; Encoding(bad) <- "UTF-8"
tryCatch(chartr("a", "b", bad), error = function(e) print(conditionMessage(e)))
tryCatch(chartr("a", "b", c("ok", "a\xff")), error = function(e) print(conditionMessage(e)))
