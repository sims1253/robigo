# structure(), which base writes with `attributes<-`
s <- structure(1:6, dim = 2:3, foo = "bar")
attributes(s)
s2 <- structure(list(1), class = "zz")
oldClass(s2)
s3 <- structure(1:2, .Names = c("a", "b"))
s3
s4 <- structure(1:4, .Dim = c(2L, 2L), .Dimnames = list(c("a", "b"), NULL))
attributes(s4)
s5 <- structure(c(a = 1), names = NULL)
s5
