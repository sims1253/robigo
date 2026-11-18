# gram.y NumericValue's integer-literal warnings (an L literal that is not an
# integer, has a decimal, or has an unnecessary decimal point), raised by
# the top-level parse (no call) and by parse().
x <- 2147483648L
x
y <- 1.5L
z <- 1e-3L
w <- 1.0L
v <- 0x10L
u <- 1e3L
t <- 2147483647L
s <- -2147483648L
r <- 0x80000000L
q <- 1e10L
p <- 0xFFFFFFFFFL
o <- 1.L
typeof(x); typeof(u); typeof(t); typeof(s); typeof(r); typeof(o)
e <- parse(text = "a <- 3000000000L")
e
f <- function() 4294967296L
f()
