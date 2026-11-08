# needs: tibble pillar
# Pillar formatting: widths, truncation, type abbreviations, list and
# nested columns at a fixed console width.
suppressPackageStartupMessages(library(tibble))
options(width = 60)
print(tibble(id = 1:12, val = seq(0.125, by = 0.5, length.out = 12),
             txt = sprintf("row-%02d", 1:12), lst = lapply(1:12, function(i) 1:i)))
print(tibble(big = c(1e12, 2.5e-8, NA), when = as.Date(c("2026-01-02", "1999-12-31", NA))))
