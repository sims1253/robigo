# `attr<-` and setAttrib: replacing keeps an attribute's place, NULL
# removes it, the special names go through their setters
x <- c(b = 2, a = 1)
attr(x, "z") <- 1
attr(x, "a") <- 2
attr(x, "z") <- 3
attributes(x)
attr(x, "z") <- NULL
attributes(x)
attr(x, "names") <- c("p", "q")
x
attr(x, "names") <- NULL
x
m <- 1:6
attr(m, "dim") <- c(a = 2, b = 3)
attributes(m)
attr(m, "dimnames") <- list(c("r", "s"), NULL)
attributes(m)
w <- 1:2
attr(w, "names") <- c(x = "p")
attributes(w)
f <- function(a) a
attr(f, "note") <- "s"
attributes(f)
f(5)
g <- f
attr(g, "note") <- NULL
attributes(f)
attributes(g)
q <- quote(f(1, 2))
attr(q, "foo") <- "bar"
attributes(q)
`attr<-`(1:2, value = "v", which = "w")
`attr<-`(1:2, "wh", "v")
