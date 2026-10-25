# `[<-`, `[[<-` and `$<-` on lists: deletion, extension, nesting.
l <- list(a = 1, b = "x", c = TRUE)
l$d <- 4
l[["e"]] <- 5:6
l["f"] <- list(NULL)
names(l)
l$b <- NULL
names(l)
l[["c"]] <- NULL
names(l)
l[c("a", "d")] <- NULL
names(l)
l
l$zz <- NULL
names(l)
l[["zz"]] <- NULL
names(l)
# numeric deletion and NULL padding
p <- list(1, 2, 3, 4)
p[[2]] <- NULL
length(p)
p[c(1, 3)] <- NULL
p
p[[5]] <- "five"
p
p[7] <- list(7)
length(p)
p[10] <- NULL
length(p)
p[[20]] <- NULL
length(p)
# [<- with lists and atomic values
q <- list(1, 2, 3)
q[2:3] <- c("two", "three")
q
q[1] <- list(c(1, 1))
q[[1]]
q[c(TRUE, FALSE, TRUE)] <- list(NULL)
q
q[2] <- list(list(9))
str_q <- q[[2]]
str_q
# [[<- puts the value itself into the list
r <- list()
r[["v"]] <- 1:3
r[["w"]] <- list(a = 1)
r$x <- NULL
r
r[[1]][2] <- 20L
r$w$a <- "A"
r$w$b$c <- "deep"
r
r[["w"]][["b"]][["c"]]
# recursive [[<-
nest <- list(a = list(b = list(c = 1)))
nest[[c("a", "b", "c")]] <- 2
nest$a$b$c
nest[[c(1, 1, 2)]] <- "new"
nest$a$b
nest[[c("a", "b")]] <- NULL
nest
# names(x)[i] <- and friends through nested replacement functions
w <- list(1, 2)
names(w)[2] <- "second"
names(w)
attr(w, "tag") <- "t"
attr(w, "tag")[2] <- "u"
attr(w, "tag")
# atomic LHS becomes a list
v <- 1:3
v[[2]] <- list("z")
v
v2 <- c(a = 1, b = 2)
v2[["b"]] <- list(c(1, 2))
v2
# assigning NULL into atomic vectors
v3 <- 1:3
v3[2] <- list(NULL)
v3
# $<- on NULL creates a list
n <- NULL
n$a <- 1
n$b <- "b"
n
# copies are independent
orig <- list(k = 1)
cpy <- orig
cpy$k <- 2
orig$k
cpy$k
