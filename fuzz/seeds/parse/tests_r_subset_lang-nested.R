# Recursive [[<- and nested replacement on calls and pairlists.
e <- quote(f(g(x, y), z))
e[[c(2, 1)]] <- as.name("h")
e
e[[c(2, 3)]] <- NULL
e
e[[c(2, 2)]] <- quote(w + 1)
e
e[[c(2, 3)]] <- 5
e
e[[2]][[1]] <- quote(k)
e
e[[2]]$named <- TRUE
e
e[[3]] <- quote(q)
e
l <- list(call = quote(f(a)))
l$call[[2]] <- 99
l
l[[c(1, 1)]] <- as.name("ff")
l$call
pl <- pairlist(a = pairlist(b = 1))
pl[[c(1, 1)]] <- 2
pl$a$b
f <- function(x) x
body_e <- quote({ a; b })
body_e[[2]] <- quote(c)
body_e
length(body_e)
body_e[[4]] <- quote(d)
body_e
cl <- call("round", 10.5)
cl[[2]]
cl[[3]] <- 1
cl
names(cl)
cl[["digits"]] <- 2
names(cl)
cl
