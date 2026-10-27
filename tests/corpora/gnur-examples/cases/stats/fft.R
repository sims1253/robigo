### Name: fft
### Title: Fast Discrete Fourier Transform (FFT)
### Aliases: fft mvfft
### Keywords: math dplot

### ** Examples

x <- 1:4
fft(x)
fft(fft(x), inverse = TRUE)/length(x)

## Slow Discrete Fourier Transform (DFT) - e.g., for checking the formula
fft0 <- function(z, inverse=FALSE) {
  n <- length(z)
  if(n == 0) return(z)
  k <- 0:(n-1)
  ff <- (if(inverse) 1 else -1) * 2*pi * 1i * k/n
  vapply(1:n, function(h) sum(z * exp(ff*(h-1))), complex(1))
}

relD <- function(x,y) 2* abs(x - y) / abs(x + y)
n <- 2^8
z <- complex(n, rnorm(n), rnorm(n))
## No test: 
##D ## relative differences in the order of 4*10^{-14} :
##D summary(relD(fft(z), fft0(z)))
##D summary(relD(fft(z, inverse=TRUE), fft0(z, inverse=TRUE)))
## End(No test)


