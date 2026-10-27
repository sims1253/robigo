out <- character(20000)
for (i in seq_along(out)) out[i] <- paste0("item-", i, "-", i %% 7)
print(length(unique(out)))
print(sum(nchar(gsub("-", "", out))))
