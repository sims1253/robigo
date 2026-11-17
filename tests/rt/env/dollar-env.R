# `$` of an environment: R_subset3_dflt's lookup of installTrChar(name), in
# compiled loops and at top level.
e <- new.env()
e$f <- function(x) x + 1
e$v <- 10
s <- 0
g <- function() { for (i in 1:3) s <- s + e$f(i) + e$v; s }
g()
h <- function() list(e$missing, is.null(e$missing))
h()
delayedAssign("p", { cat("forced p\n"); 3 }, assign.env = e)
k <- function() c(e$p, e$p)
k()
makeActiveBinding("a", local({ n <- 0; function() { n <<- n + 1; n } }), e)
m <- function() c(e$a, e$a, e$a)
m()
e$`café` <- "non-ASCII"
n <- function() e$`café`
n()
e$`with space` <- "quoted"
q <- function() e$`with space`
q()
# a partial name is no match in an environment
r <- function() is.null(e$fu)
r()
# an environment with a class dispatches first
ce <- structure(new.env(), class = "myenv")
ce$z <- 1
`$.myenv` <- function(x, name) paste("myenv", name)
t <- function() ce$z
t()
rm(`$.myenv`)
t()
# the base and global environments
u <- function() is.function(baseenv()$paste)
u()
gv <- "global value"
w <- function() globalenv()$gv
w()
