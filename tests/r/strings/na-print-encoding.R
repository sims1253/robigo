# na.print: numbers print the NA string's bytes padded by bytes; string
# NAs escape them as native text and pad by their Rstrlen width (UTF-8
# locale first, then C).
u <- intToUtf8(233)
print(matrix(c(NA, 1L), 1), na.print = u)
print(c(NA, 2.5), na.print = paste0(u, u))
print(c(NA, TRUE), na.print = u)
print(c(x = NA_character_, y = "ab"), na.print = "a\tb")
print(c(NA, 1.5), na.print = "a\\b")
invisible(Sys.setlocale("LC_CTYPE", "C"))
m <- matrix(c(NA_character_, "ab"), 1, 2)
print(m, na.print = u)
print(m, na.print = u, quote = FALSE)
print(c(a = NA_character_, b = "xyz"), na.print = paste0(u, u))
print(matrix(1:2, 1, dimnames = list(NA, c(NA, "b"))), na.print = u)
print(matrix(c(NA, 1L), 1), na.print = u)
