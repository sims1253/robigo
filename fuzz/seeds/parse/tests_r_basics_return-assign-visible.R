# GNU R's compiled code returns the value of `return(s <- v)` visibly (the
# bytecode RETURN does not keep the assignment's invisibility); robigo returns
# it invisibly in every tier.
h <- function() { s <- 1L; for (i in 1:3) if (i == 2L) return(s <- s + 1L) }
h(); h(); h()
g <- function() return(x <- 5)
g(); g(); g()
