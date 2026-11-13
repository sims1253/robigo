# cat's fill: lines broken at the fill width (TRUE: width 80), labels at
# the start of each line, a final newline.
cat(1:30, fill = 30)
cat(1:30, fill = 30, labels = paste0("{", 1:5, "}:"))
cat(paste0("word", 1:40), fill = TRUE)
cat(1:5, fill = TRUE)
cat("a", "b", fill = 1)
