p <- new.env()
reg.finalizer(p, function(e) cat("plain at exit\n"))
e <- new.env()
e$id <- "exit"
reg.finalizer(e, function(e) cat("onexit:", e$id, "\n"), onexit = TRUE)
f <- new.env()
f$id <- "dead"
reg.finalizer(f, function(f) cat("before exit:", f$id, "\n"))
rm(f)
invisible(gc())
cat("body\n")
