e <- new.env()
e$id <- "error exit"
reg.finalizer(e, function(e) cat("onexit:", e$id, "\n"), onexit = TRUE)
cat("body\n")
stop("boom")
