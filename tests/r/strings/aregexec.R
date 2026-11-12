# utils::aregexec: approximate regexec through TRE (agrep.c:do_aregexec)
x <- c("1 lazy", "1", "1 LAZY", NA, "lazzy dog")
m <- utils::aregexec("laysy", x, max.distance = 2)
print(m)
print(regmatches(x, m))
# subexpressions
print(utils::aregexec("(lay)(sy)", x, max.distance = 2))
print(utils::aregexec("(a)|(b)", c("xa", "xb", "zz"), max.distance = 0))
print(utils::aregexec("la[sz]y", x, max.distance = 1, ignore.case = TRUE))
# fixed ignores ignore.case, with a warning
r <- withCallingHandlers(utils::aregexec("lasy", x, fixed = TRUE, ignore.case = TRUE),
                         warning = function(w) {
                           cat("warning:", conditionMessage(w), "\n")
                           invokeRestart("muffleWarning")
                         })
print(r)
# max.distance as a list of bounds
print(utils::aregexec("lasy", x, max.distance = list(all = 1, ins = 0)))
print(utils::aregexec("lasy", x, max.distance = list(cost = 1), costs = c(sub = 2)))
# offsets are characters, or bytes with useBytes
print(utils::aregexec("fünf", c("ein fünfzig", "funf"), max.distance = 1))
print(utils::aregexec("fünf", c("ein fünfzig", "funf"), max.distance = 1,
                      useBytes = TRUE))
# only the first pattern is used
r <- withCallingHandlers(utils::aregexec(c("a", "b"), "xa"),
                         warning = function(w) {
                           cat("warning:", conditionMessage(w), "\n")
                           invokeRestart("muffleWarning")
                         })
print(r)
print(tryCatch(utils::aregexec("a(", "a"), error = function(e) conditionMessage(e)))
print(tryCatch(utils::aregexec(NA_character_, "a"), error = function(e) conditionMessage(e)))
print(tryCatch(utils::aregexec("", "a"), error = function(e) conditionMessage(e)))
