# R 4.6's base `substring`/`substring<-` default `last = NULL`; R 4.5's
# default is 1000000L. Both mean "to the end".
substring("abcdef", 2)
substring(c("abc", "defgh"), 2)
substring(c("abcdef", "ghijkl", "mnopqr"), c(2, 3, 1))
substring(c("abcdef", "ghijkl"), 2)
substring("abcdef", c(2, 4))
substring(c("abc", NA, "def"), 2)
x <- "abcdef"
substring(x, 3) <- "XY"
x
x <- c("abcdef", "ghijkl")
substring(x, c(2, 3)) <- c("X", "Y")
x
