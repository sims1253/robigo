# GNU R's byte-code compiler keeps one constant per identical() value, and
# identical(0, -0) is TRUE: in a compiled function a literal -0 (folded
# from `-0`) becomes the 0 already in its constant pool. Interpreted code
# keeps -0. (A gap: robigo's compiled code keeps -0.)
f <- function() { a <- 0; b <- -0; for (i in 1:2) v <- b; 1 / v }
print(f())
g <- function() { b <- -0; for (i in 1:2) v <- b; 1 / v }
print(g())
