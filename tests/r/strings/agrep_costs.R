# agrep and agrepl through TRE (agrep.c:do_agrep): regular expressions, cost
# vectors and max.distance lists
x <- c("1 lazy", "1", "1 LAZY", NA, "lazzy dog", "lasy")
print(agrep("laysy", x, max.distance = 2))
print(agrep("la[sz]y", x, max.distance = 1, fixed = FALSE))
print(agrep("la[sz]y", x, max.distance = 1, fixed = FALSE, value = TRUE))
print(agrepl("LA.Y", x, max.distance = 0, fixed = FALSE, ignore.case = TRUE))
# max.distance lists: cost, ins, del, sub and all, as counts or fractions
print(agrep("lasy", x, max.distance = list(cost = 2), costs = c(ins = 1, del = 3, sub = 1)))
print(agrep("lasy", x, max.distance = list(sub = 0, all = 1)))
print(agrep("lasy", x, max.distance = list(all = 0.3, ins = 0, del = 0)))
print(agrep("lasy", x, max.distance = list(cost = 0.25), costs = c(sub = 2)))
# cost vectors and lists
print(agrepl("lasy", x, max.distance = 1, costs = list(sub = 2)))
print(agrep("lasy", x, max.distance = 2, costs = c(1, 1, 3), value = TRUE))
# a free substitution, and case folding through TRE's tags
print(agrepl("ab", "aa", max.distance = list(all = 1), costs = c(sub = 0)))
print(agrepl("ab", "aa", max.distance = list(all = 1), costs = c(sub = 0), ignore.case = TRUE))
print(agrepl("ab", "aa", max.distance = list(all = 2), costs = c(sub = 0), ignore.case = TRUE))
print(agrepl("abc", c("abc", "abd", "xabcx"), max.distance = 0))
# NA patterns, names
print(agrep(NA, c(a = "x", b = "y"), value = TRUE))
print(agrepl(NA, c("x", "y")))
print(agrep("lasy", c(p = "1 lazy", q = "1", r = "lasy"), value = TRUE))
# multibyte strings, as characters or bytes
print(agrep("fünf", c("ein fünfzig", "funf", "fnf"), max.distance = 1))
print(agrep("fünf", c("ein fünfzig", "funf", "fnf"), max.distance = 1, useBytes = TRUE))
# a bound past the integer range becomes unlimited, with a warning
r <- withCallingHandlers(agrepl("lasy", x, max.distance = 3e9),
                         warning = function(w) {
                           cat("warning:", conditionMessage(w), "\n")
                           invokeRestart("muffleWarning")
                         })
print(r)
print(tryCatch(agrep("a(", "a", fixed = FALSE), error = function(e) conditionMessage(e)))
print(tryCatch(agrep("", "a"), error = function(e) conditionMessage(e)))
