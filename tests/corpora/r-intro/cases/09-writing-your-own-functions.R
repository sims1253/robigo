## An Introduction to R (GNU R 4.5.0), chapter: Writing your own functions
## The manual's runnable examples, from doc/manual/R-intro.texi in the R
## sources (part of R, GPL-2 or later). Examples that need a terminal,
## a browser, the network or a data file are left out (see README.md).
options(keep.source = TRUE, useFancyQuotes = FALSE, warn = 1)
set.seed(123)
name <- function(arg_1, arg_2, ...) expression


## ---- Simple examples
twosam <- function(y1, y2) {
    n1  <- length(y1); n2  <- length(y2)
    yb1 <- mean(y1);   yb2 <- mean(y2)
    s1  <- var(y1);    s2  <- var(y2)
    s <- ((n1-1)*s1 + (n2-1)*s2)/(n1+n2-2)
    tst <- (yb1 - yb2)/sqrt(s*(1/n1 + 1/n2))
    tst
  }

bslash <- function(X, y) {
  X <- qr(X)
  qr.coef(X, y)
}


## ---- Defining new binary operators
"%!%" <- function(X, y) { ... }


## ---- Named arguments and defaults
fun1 <- function(data, data.frame, graph=TRUE, limit=20) { ... }


## ---- More advanced examples
bdeff <- function(blocks, varieties) {
    blocks <- as.factor(blocks)             # minor safety move
    b <- length(levels(blocks))
    varieties <- as.factor(varieties)       # minor safety move
    v <- length(levels(varieties))
    K <- as.vector(table(blocks))           # remove dim attr
    R <- as.vector(table(varieties))        # remove dim attr
    N <- table(blocks, varieties)
    A <- 1/sqrt(K) * N * rep(1/sqrt(R), rep(b, v))
    sv <- svd(A)
    list(eff=1 - sv$d^2, blockcv=sv$u, varietycv=sv$v)
}

no.dimnames <- function(a) {
  ## Remove all dimension names from an array for compact printing.
  d <- list()
  l <- 0
  for(i in dim(a)) {
    d[[l <- l + 1]] <- rep("", i)
  }
  dimnames(a) <- d
  a
}

area <- function(f, a, b, eps = 1.0e-06, lim = 10) {
  fun1 <- function(f, a, b, fa, fb, a0, eps, lim, fun) {
    ## function `fun1' is only visible inside `area'
    d <- (a + b)/2
    h <- (b - a)/4
    fd <- f(d)
    a1 <- h * (fa + fd)
    a2 <- h * (fd + fb)
    if(abs(a0 - a1 - a2) < eps || lim == 0)
      return(a1 + a2)
    else {
      return(fun(f, a, d, fa, fd, a1, eps, lim - 1, fun) +
             fun(f, d, b, fd, fb, a2, eps, lim - 1, fun))
    }
  }
  fa <- f(a)
  fb <- f(b)
  a0 <- ((fa + fb) * (b - a))/2
  fun1(f, a, b, fa, fb, a0, eps, lim, fun1)
}


## ---- Scope
f <- function(x) {
  y <- 2*x
  print(x)
  print(y)
  print(z)
}

cube <- function(n) {
  sq <- function() n*n
  n*sq()
}


## ---- Customizing the environment
.Last <- function() {
  graphics.off()                        # a small safety measure.
  cat(paste(date(),"\nAdios\n"))        # Is it time for lunch?
}


## ---- Classes, generic functions and object orientation
methods(class="data.frame")

methods(plot)

coef

methods(coef)

getAnywhere("coef.aov")
getS3method("coef", "aov")
