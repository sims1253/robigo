# x[[i, j]] <- y on a pairlist with dims (subassign.c:do_subassign2_dflt's
# pairlist branch) and length(<pairlist>) <- n (builtin.c:xlengthgets),
# from GNU R's reg-tests-1c and reg-tests-1d.
a <- pairlist(10, 20, 30, 40, 50, 60)
dim(a) <- c(2, 3)
dimnames(a) <- list(c("a", "b"), c("x", "y", "z"))
a[["a", "x"]] <- 0
print(a[["a", "x"]] == 0)
a[[2, 3]] <- "s"; print(a[[2, 3]]); print(typeof(a))
a[[1, 2]] <- NULL; print(length(a)); print(a[[1, 2]])
r <- tryCatch({a[[3, 1]] <- 1}, error = function(e) e); print(r); print(class(r))
r <- tryCatch({a[["q", 1]] <- 1}, error = function(e) e); print(r)
r <- tryCatch({a[[1, 1, 1]] <- 1}, error = function(e) e); print(r)
r <- tryCatch({a[[1:2, 1]] <- 1}, error = function(e) e); print(r)
print(attributes(a))
# The pairlist branch reads the dimnames without checking for NULL.
b <- pairlist(a = 1, b = 2); dim(b) <- c(1, 2)
r <- tryCatch({b[[1, 2]] <- 5}, error = function(e) e); print(r)
dimnames(b) <- list(NULL, c("u", "v"))
b[[1, "v"]] <- 5; print(b[[1, 2]]); print(names(b))
# Tags stay; length<- adds no names attribute to a pairlist.
pl <- pairlist(a = 1, b = 2); length(pl) <- 1
print(identical(pl, pairlist(a = 1))); print(attributes(pl))
al <- formals(ls); length(al) <- 2
print(identical(al, as.pairlist(alist(name = , pos = -1L))))
pl <- pairlist(a = 1, b = 2); length(pl) <- 4; print(names(pl)); print(attributes(pl))
x <- c(a = 1, b = 2); length(x) <- 3; print(x)
x <- pairlist(list(1, 2))
x[[c(1, 2)]] <- NULL
print(identical(x, pairlist(list(1))))
