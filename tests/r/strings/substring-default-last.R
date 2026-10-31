# R 4.6's base `substring`/`substring<-` default `last = NULL`; R 4.5's
# default is 1000000L. Both mean "to the end".
substring("abcdef", 2)
substring(c("abc", "defgh"), 2)
x <- "abcdef"
substring(x, 3) <- "XY"
x
