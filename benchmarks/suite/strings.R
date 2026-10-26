# String building and processing.
n <- 50000
ids <- paste0("item_", 1:n, "_", (1:n) %% 97)
up <- toupper(ids)
parts <- strsplit(ids[1:20000], "_", fixed = TRUE)
nums <- vapply(parts, function(p) as.integer(p[2]), 0L)
g <- gsub("_([0-9]+)$", "-\\1", ids)
cnt <- sum(nchar(g)) + sum(grepl("item_1", ids)) + sum(nums) + length(unique(substr(up, 1, 7)))
s <- sprintf("%05d:%s", 1:20000, ids[1:20000])
cnt <- cnt + sum(nchar(s))
cat(cnt, "\n")
