# cat's sep: recycled over the elements, "" for none, a newline in any
# sep adds a final newline.
cat("a", "b", "c", sep = "")
cat("\n")
cat(1:3, sep = ", ")
cat("\n")
cat("a", "b", "c", sep = "-+")
cat("\n")
cat("multi\nline", "x", sep = "\n")
cat(1:5, sep = "\n")
