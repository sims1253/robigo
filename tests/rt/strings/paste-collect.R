# paste() long enough to collect inside one call (Cx::fill_strings) under
# gctorture, or the young-moving collector with it: a collection runs between
# two elements, and each element is made again from the arguments' roots (an
# object's as.character() value among them) into a result read again from
# its root. The values are GNU R's.
x <- paste0("u", 1:2000)
length(x); x[c(1, 2, 999, 2000)]
sum(nchar(x)); length(unique(x))
y <- paste("a", 1:5000, c("x", "y"), sep = "-")
y[c(1, 2, 4999, 5000)]; sum(nchar(y))
f <- structure(1:3000, class = "num3")
as.character.num3 <- function(x, ...) sprintf("n%04d", unclass(x))
z <- paste(f, rev(f), sep = "/")
z[c(1, 1500, 3000)]; length(unique(z))
w <- paste0("v", 1:3000, collapse = "")
nchar(w); substr(w, 1, 20)
e <- paste0("é", 1:2000)
Encoding(e[c(1, 2000)]); e[2000]
paste0(character(0), 1:3)
paste0("only", collapse = NULL)
