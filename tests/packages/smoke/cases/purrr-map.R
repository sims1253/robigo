# needs: purrr magrittr
# Iteration indices and dots written by C before each callback, read
# back in R inside the same call.
suppressPackageStartupMessages({library(purrr); library(magrittr)})
print(map_int(1:5, function(z) z * z))
print(map2_chr(c("a", "b"), 1:2, function(s, i) paste0(s, i)))
print(imap_chr(c(x = "p", y = "q"), function(v, nm) paste0(nm, "=", v)))
print(1:4 %>% map_dbl(function(z) z / 2) %>% sum())
print(accumulate(1:4, `+`))
print(keep(1:6, function(z) z %% 2 == 0))
print(tryCatch(map_int(1:3, function(z) if (z == 2) stop("two") else z),
               error = function(e) conditionMessage(e)))
