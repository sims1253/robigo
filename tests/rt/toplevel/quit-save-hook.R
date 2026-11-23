# quit("yes") saves through the global environment's sys.save.image.
sys.save.image <- function(file) cat("saving", file, "\n")
quit("yes", 4, FALSE)
