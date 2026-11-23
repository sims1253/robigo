# x[i] <- NA into complex stores NA in both parts (subassign.c VectorAssign
# 1510/1513/1514); coerceVector gives a zero imaginary part instead.
y <- c(1+1i, 2+2i); y[2] <- NA; Im(y)
y <- c(1+1i, 2+2i); y[2] <- NA_integer_; Im(y)
y <- c(1+1i, 2+2i); y[2] <- NA_real_; Im(y)
y <- c(1+1i, 2+2i); y[2] <- NaN; Im(y)
y <- c(1+1i, 2+2i); y[[2]] <- NA; Im(y)
m <- matrix(1i, 2, 2); m[1, ] <- NA; Im(m)
a <- array(1i, c(2, 2, 2)); a[1, 1, ] <- NA; Im(a)
Im(c(1i, NA))
Im(as.complex(NA))
x <- cbind(x1 = 3 + 2i, x2 = c(4:1, 2:5) - 5i)
x[3, ] <- NA; x[4, 2] <- NA
rowMeans(x, na.rm = TRUE); colMeans(x, na.rm = TRUE)
