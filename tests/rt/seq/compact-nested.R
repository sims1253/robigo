# Compact sequences inside lists that closures build (lapply): the walkers
# read them as GNU R does: unlist and c through INTEGER (bind.c), identical,
# unique, duplicated, match and anyNA by element, vapply copying the data.
mk <- function() lapply(1:3, function(i) i:(i + 2L))
print(deparse(mk())); dput(1:10); dput(10:1); dput(-0.5:2.5); dput(list(a = 1:3, b = seq_len(0)))
print(unlist(mk())); print(unlist(mk(), recursive = FALSE)); print(c(mk(), recursive = TRUE)); print(c(mk(), 1:2))
print(identical(mk(), mk())); print(identical(mk(), list(1:3, 2:4, 3:5))); print(unique(c(mk(), mk())))
print(duplicated(c(mk(), list(1:3)))); print(anyNA(mk(), recursive = TRUE)); print(lengths(mk()))
print(c(vapply(1:3, function(i) i:(i+1L), integer(2)))); print(c(vapply(1:2, function(i) i:(i+2L), numeric(3))))
print(paste(mk())); print(as.character(mk())); print(match(list(2:4), mk()))
e <- new.env(); assign("a", 1:3, envir = e); print(unlist(as.list(e)))
print(unlist(list(a = 1:2, b = list(c = 3:4, d = 5:6))))
