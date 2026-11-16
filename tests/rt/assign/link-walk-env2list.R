# The link-walk family (link-walk.R): as.list(<environment>) and mget store
# each value in their answer as they get it (FrameValues', do_mget's
# SET_VECTOR_ELT), so a promise forced later that writes an earlier value
# copies it: the answer keeps 1 2, the binding has 99 2.
e <- new.env(hash = FALSE)
delayedAssign("b", { a[1] <- 99; 0 }, eval.env = e, assign.env = e)
assign("a", c(1, 2), envir = e)
print(ls(e, sorted = FALSE))
l <- as.list(e, sorted = TRUE)
print(l$a)
print(e$a)
e <- new.env(hash = FALSE)
delayedAssign("b", { a[1] <- 99; 0 }, eval.env = e, assign.env = e)
assign("a", c(1, 2), envir = e)
l <- as.list(e)
print(l$a)
print(names(l))
print(length(as.list(new.env())))
print(names(as.list(new.env())))
e <- new.env()
assign("a", c(1, 2), envir = e)
delayedAssign("b", { a[1] <- 99; 0 }, eval.env = e, assign.env = e)
l <- mget(c("a", "b"), envir = e)
print(l$a)
print(e$a)
print(names(l))
f <- function(nm) { e$a[2] <- -1; 0 }
assign("a", c(1, 2), envir = e)
print(mget(c("a", "zz"), envir = e, ifnotfound = list(f)))
