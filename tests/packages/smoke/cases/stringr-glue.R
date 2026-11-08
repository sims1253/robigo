# needs: stringr glue
suppressPackageStartupMessages({library(stringr); library(glue)})
words <- c("alpha_1", "BETA-22", "gamma33")
print(str_match(words, "^([a-z]+)[_-]?(\\d*)$"))
print(str_replace_all(words, "[0-9]+", function(m) paste0("<", m, ">")))
print(str_c(words, collapse = ", "))
who <- "smoke"
print(glue("hello {who}, {1 + 1} items, {toupper(who)}"))
print(glue_data(list(a = 1:3, b = letters[1:3]), "{a}={b}"))
