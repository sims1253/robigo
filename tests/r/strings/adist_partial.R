# utils::adist(partial = TRUE): each x[i] is a TRE pattern matched
# approximately inside each y[j] (agrep.c:do_adist, tre_regaexec).
print(utils::adist("lasy", "1 lazy 2", partial = TRUE))
# counts and offsets, names, NA rows and cells
d <- utils::adist(c(a = "lasy", b = "laser", c = NA),
                  c(x = "1 lazy 2", y = "lazer beams", z = NA, w = "nothing"),
                  partial = TRUE, counts = TRUE)
print(d)
print(names(attributes(d)))
print(utils::adist("kitten", c("sitting", "a kitten here", "KITTEN"),
                   partial = TRUE, ignore.case = TRUE))
# costs weight TRE's choice of edits
print(utils::adist("ab", "xaby", partial = TRUE, costs = c(ins = 2, del = 3, sub = 5),
                   counts = TRUE))
print(utils::adist("ab", c("b", "xab", "a"), partial = TRUE, costs = c(1, 1, 1),
                   counts = TRUE))
# fixed = FALSE: x is a regular expression
print(utils::adist("a.c", c("abc", "a.c", "xyz"), fixed = FALSE, counts = TRUE))
print(utils::adist("a.c", c("abc", "a.c", "xyz"), fixed = TRUE, partial = TRUE))
print(utils::adist("(a)(b)", c("ab", "xb"), fixed = FALSE, counts = TRUE))
# multibyte strings: characters, or bytes with useBytes
print(utils::adist(c("über", "fünf"), c("Über alles", "fünfzig", "funf"),
                   partial = TRUE, counts = TRUE))
print(utils::adist(c("über", "fünf"), c("Über alles", "fünfzig", "funf"),
                   partial = TRUE, useBytes = TRUE, counts = TRUE))
print(utils::adist(character(), "x", partial = TRUE, counts = TRUE))
# the pattern must compile
r <- tryCatch(utils::adist("a(", "a", partial = TRUE, fixed = FALSE),
              error = function(e) conditionMessage(e))
print(r)
