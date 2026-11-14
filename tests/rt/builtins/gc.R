e <- new.env()
.Internal(reg.finalizer(e, function(e) cat("finalized\n"), FALSE))
e <- NULL
invisible(.Internal(gc(FALSE, FALSE, TRUE)))
length(.Internal(gc(FALSE, FALSE, TRUE)))
