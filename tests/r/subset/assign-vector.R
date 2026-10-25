# `[<-` and `[[<-` on atomic vectors: promotion, extension, names, recycling.
x <- 1:5
x[2] <- 20L
x
x[c(1, 5)] <- c(-1L, -5L)
x
x[-1] <- 0L
x
x[x == 0L] <- 9L
x
x[c(TRUE, FALSE)] <- 1L
x
x[] <- 7L
x
x[NA] <- 3L
x
x[c(NA, 2)] <- 4L
x
x[integer(0)] <- 100L
x
# type promotion
i <- 1:3
i[2] <- 2.5
i
typeof(i)
lg <- c(TRUE, FALSE)
lg[1] <- 5L
lg
typeof(lg)
lg2 <- c(TRUE, NA)
lg2[2] <- 1i
lg2
r <- c(1.5, 2.5)
r[1] <- "a"
r
ch <- c("a", "b")
ch[2] <- 3.25
ch
ch[3] <- TRUE
ch
n <- c(1, 2)
n[2] <- NA
n
typeof(n)
z <- 1:3
z[0] <- 1.5
typeof(z)
# extension
e <- 1:3
e[6] <- 6L
e
e[[8]] <- 8L
e
s <- c("a")
s[3] <- "c"
s
lg3 <- TRUE
lg3[3] <- FALSE
lg3
nul <- NULL
nul[3] <- 1L
nul
nul2 <- NULL
nul2[["k"]] <- "v"
nul2
nul3 <- NULL
nul3[2] <- list(1)
nul3
# names
nm <- c(a = 1, b = 2)
nm["c"] <- 3
nm
nm[c("a", "d", "d")] <- c(10, 40, 41)
nm
nm[["e"]] <- 5
nm
nm[7] <- 7
nm
un <- 1:2
un["z"] <- 9L
un
names(nm)[2] <- "B"
nm
names(un) <- NULL
un
# recycling
rc <- 1:6
rc[1:6] <- c(0L, 1L)
rc
rc[1:4] <- 1:3
rc
rc[c(TRUE, FALSE)] <- 100L
rc
# logical subscripts longer than x extend it
ll <- 1:2
ll[c(FALSE, FALSE, TRUE)] <- 3L
ll
# assignment keeps other attributes
at <- c(u = 1, v = 2)
attr(at, "note") <- "kept"
at[2] <- 22
attributes(at)
at[["w"]] <- 3
attributes(at)
# [[<- on atomic vectors
aa <- c(1, 2, 3)
aa[[2]] <- 5
aa
aa[[2]] <- "five"
aa
bb <- 1:3
bb[[2]] <- list(9)
bb
ra <- as.raw(1:3)
ra[2] <- as.raw(255)
ra
ra[[5]] <- as.raw(7)
ra
cx <- c(1+1i)
cx[2] <- 3L
cx
cx[[3]] <- 2.5
cx
