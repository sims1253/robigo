# Subsetting environments, calls and pairlists.
e <- new.env()
e$a <- 1
e[["b"]] <- "two"
e$a
e[["b"]]
e$zz
e[["zz"]]
sort_names <- function(env) {
  n <- ls(env)
  n
}
sort_names(e)
e$a <- NULL
is.null(e$a)
exists("a", envir = e, inherits = FALSE)
f <- function(env) env$counter <- env$counter + 1
e$counter <- 0
f(e)
f(e)
e$counter
delayedAssign("lazy", {cat("forced\n"); 42}, assign.env = e)
e$lazy
e[["lazy"]]
# calls
cl <- quote(fun(x, y = 2, 3))
cl[[1]]
cl[[3]]
cl$y
cl[-1]
cl[2:3]
cl[c(1, 3)]
cl[["y"]]
cl[[1]] <- as.name("other")
cl
cl[[2]] <- quote(z + 1)
cl
cl$y <- 10
cl
cl$w <- "new"
cl
cl[[6]] <- TRUE
cl
cl[["y"]] <- NULL
cl
cl[4] <- list(quote(q))
cl
cl[c(2, 3)] <- NULL
cl
length(cl)
cl2 <- quote(g(a))
cl2[[1]] <- NULL
cl2
typeof(cl2)
cl3 <- quote(h(a, b))
cl3[-1]
typeof(cl3[1])
cl3[0]
# formula calls
fo <- quote(y ~ x + z)
fo[[2]]
fo[[3]]
fo[[3]][[2]]
# pairlists
pl <- pairlist(a = 1, b = "b")
pl$a
pl[["b"]]
pl[2]
typeof(pl[2])
pl$c <- 3
pl$c
typeof(pl)
pl[["a"]] <- NULL
names(pl)
pl[1] <- 100
typeof(pl)
pl
