# Cases adapted from GNU R's tests/reg-tests-1b.R and reg-tests-1d.R.
x <- c(a=2, b=3)
x[[quote(b)]] <- pi
identical(2, x[[quote(a)]])
identical(x, c(a=2, b=pi))
x1 <- x0 <- x <- n0 <- numeric(); x0[] <- character(); x1[1[0]] <- character()
x[] <- numeric()
identical(x0, character()); identical(x1, x0); identical(x, n0)
x[1] <- numeric(); identical(x, n0)
NUL <- NULL
NUL[3] <- integer(0); NUL[,2] <- character() ; NUL[3,4,5] <- list()
is.null(NUL)
NUL <- NULL; NUL[character()] <- "A"
identical(NUL, character())
e0 <- logical(0); dim(e0) <- c(0L, 4L)
ec <- e0
ec[,1:2] <- list()
x <- 1[0]; x[1:2] <- list()
identical(ec, e0); identical(x, 1[0])
a <- list( 1 ); b <- (a[[1]] <- a); identical(b, list( 1 ))
a <- list(x=1); b <- ( a$x  <-  a); identical(b, list(x=1))
xx0 <-
xx <- list(id = 1L,
           split = list(varid = 1L, breaks = NULL,
                        index = 1:3, right = TRUE, info = "s"),
           kids = list(id = 2L,
                       split = list(varid = 3L, breaks = 75,
                                    right = TRUE, info = "KS"),
                       kids = list(list(id = 3L, info = "yes"),
                                   list(id = 4L, info = "no")),
                       info = NULL),
           list(id = 5L,
                split = list(varid = 3L, breaks = 20,
                             right = TRUE, info = "4s"),
                kids = list(list(id = 6L, info = "no"),
                            list(id = 7L, info = "yes")),
                info = NULL),
           info = NULL)
xx[[1]] <- xx0[[1]]
xx[["kids"]] <- xx0[["kids"]]
xx[[2:1]] <- xx0[[2:1]] ; identical(xx, xx0)
xx[[3:1]] <- xx0[[3:1]] ; identical(xx, xx0)
xx[[c(3,3,1,2)]] <- "NO"
xx$kids$kids[[1]]$info
xx[[c(4,3,2,1)]] <- 70L
xx[[4]]$kids[[2]]
tm <- `[[<-`(call("T"), 1L, "~")
tm
is.call(tm)
N <- NULL; N[["a"]] <- 1:2; identical(N, list(a = 1:2))
N <- NULL; N[["a"]] <- 1  ; identical(N, list(a = 1))
aa <- 1:2; dim(aa) <- c(2L, 1L); dimnames(aa)[[1]] <- c("a", "b")
dimnames(aa)
a1 <- 1; dim(a1) <- c(1L, 1L); dimnames(a1)[[1]] <- "a"
dimnames(a1)
# huge real subscripts select nothing
x <- c(a = 1, b = 2)
x[1e10]
x[c(1, 3e9)]
x[[1e10 - 1e10 + 1]]
l <- list(1)
l[3e9]
y <- 1:3
y[-1e10]
y[c(-1, -1e10)]
y[2147483647]
