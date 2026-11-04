# utils::adist's full path: .Internal(adist()) ported from agrep.c:adist_full
# and do_adist. partial = TRUE (TRE's approximate regexec) is #234.
print(utils::adist("kitten", "sitting"))
print(utils::adist(c(a = "kitten", b = "sitting"), counts = TRUE))
print(utils::adist("kitten", "sitting", costs = c(ins = 2, del = 1, sub = 3), counts = TRUE))
print(utils::adist("abc", "abd", costs = c(sub = Inf), counts = TRUE))
print(utils::adist("ab", "AB", ignore.case = TRUE, counts = TRUE))
print(utils::adist(c("a", "b"), c(x = "a")))
print(utils::adist(character(0), "a"))
print(utils::adist("a", character(0), counts = TRUE))
print(utils::adist(c("lazy", "lasy", ""), c("1 lazy", "lazy", "")))
print(utils::adist("abc", "abc", useBytes = TRUE, counts = TRUE))
# an NA y: NA distances and counts, empty transcripts
b <- utils::adist("ab", c(NA, "b"), counts = TRUE)
print(b)
# numbers are compared as their character form
print(utils::adist(c(12, 123), 1234))
# fixed = FALSE, partial = FALSE warns, then the C code meets the character
# vector the wrapper did not turn into code points
r <- withCallingHandlers(
  tryCatch(utils::adist("a", "b", fixed = FALSE, partial = FALSE),
           error = function(e) conditionMessage(e)),
  warning = function(w) { cat("warning:", conditionMessage(w), "\n"); invokeRestart("muffleWarning") })
print(r)
# the .Internal's own checks: list elements are read lazily
print(tryCatch(.Internal(adist(list(1:2), "b", c(1, 1, 1), FALSE, TRUE, FALSE, FALSE, FALSE)),
               error = function(e) conditionMessage(e)))
print(.Internal(adist(list(), "b", c(1, 1, 1), FALSE, TRUE, FALSE, FALSE, FALSE)))
print(tryCatch(.Internal(adist(list(1:2), list(1L), c(1, 1, 1), NA, TRUE, FALSE, FALSE, FALSE)),
               error = function(e) conditionMessage(e)))
