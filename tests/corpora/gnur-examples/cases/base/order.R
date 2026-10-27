### Name: order
### Title: Ordering Permutation
### Aliases: order sort.list
### Keywords: univar manip

### ** Examples

require(stats)

(ii <- order(x <- c(1,1,3:1,1:4,3), y <- c(9,9:1), z <- c(2,1:9)))
## 6  5  2  1  7  4 10  8  3  9
rbind(x, y, z)[,ii] # shows the reordering (ties via 2nd & 3rd arg)

## Suppose we wanted descending order on y.
## A simple solution for numeric 'y' is
rbind(x, y, z)[, order(x, -y, z)]
## More generally we can make use of xtfrm
cy <- as.character(y)
rbind(x, y, z)[, order(x, -xtfrm(cy), z)]
## The radix sort supports multiple 'decreasing' values:
rbind(x, y, z)[, order(x, cy, z, decreasing = c(FALSE, TRUE, FALSE),
                       method="radix")]

## Sorting data frames:
dd <- transform(data.frame(x, y, z),
                z = factor(z, labels = LETTERS[9:1]))
## Either as above {for factor 'z' : using internal coding}:
dd[ order(x, -y, z), ]
## or along 1st column, ties along 2nd, ... *arbitrary* no.{columns}:
dd[ do.call(order, dd), ]

set.seed(1)  # reproducible example:
d4 <- data.frame(x = round(   rnorm(100)), y = round(10*runif(100)),
                 z = round( 8*rnorm(100)), u = round(50*runif(100)))
(d4s <- d4[ do.call(order, d4), ])
(i <- which(diff(d4s[, 3]) == 0))
#   in 2 places, needed 3 cols to break ties:
d4s[ rbind(i, i+1), ]

## rearrange matched vectors so that the first is in ascending order
x <- c(5:1, 6:8, 12:9)
y <- (x - 5)^2
o <- order(x)
rbind(x[o], y[o])

## tests of na.last
a <- c(4, 3, 2, NA, 1)
b <- c(4, NA, 2, 7, 1)
z <- cbind(a, b)
(o <- order(a, b)); z[o, ]
(o <- order(a, b, na.last = FALSE)); z[o, ]
(o <- order(a, b, na.last = NA)); z[o, ]

## No test: 
##D ##  speed examples on an average laptop for long vectors:
##D ##  factor/small-valued integers:
##D x <- factor(sample(letters, 1e7, replace = TRUE))
##D system.time(o <- sort.list(x, method = "quick", na.last = NA)) # 0.1 sec
##D stopifnot(!is.unsorted(x[o]))
##D system.time(o <- sort.list(x, method = "radix")) # 0.05 sec, 2X faster
##D stopifnot(!is.unsorted(x[o]))
##D ##  large-valued integers:
##D xx <- sample(1:200000, 1e7, replace = TRUE)
##D system.time(o <- sort.list(xx, method = "quick", na.last = NA)) # 0.3 sec
##D system.time(o <- sort.list(xx, method = "radix")) # 0.2 sec
##D ##  character vectors:
##D xx <- sample(state.name, 1e6, replace = TRUE)
##D system.time(o <- sort.list(xx, method = "shell")) # 2 sec
##D system.time(o <- sort.list(xx, method = "radix")) # 0.007 sec, 300X faster
##D ##  double vectors:
##D xx <- rnorm(1e6)
##D system.time(o <- sort.list(xx, method = "shell")) # 0.4 sec
##D system.time(o <- sort.list(xx, method = "quick", na.last = NA)) # 0.1 sec
##D system.time(o <- sort.list(xx, method = "radix")) # 0.05 sec, 2X faster
## End(No test)


