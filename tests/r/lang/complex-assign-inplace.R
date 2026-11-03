# R: eval.c:applydefine + duplicate.c:lazy_duplicate — a replacement gets
# the element of a container without copying, so `a$x[[1]] <- v` mutates
# a$x in place (no tracemem report). Aliases of the container keep the old
# value (the write-back duplicates the container spine like `$<-`).
norm <- function(x) gsub("0x[0-9a-f]+", "0xADDR", x)

lines <- character(0)
tc <- textConnection("lines", "wr")
sink(tc)
a <- list(); a$x <- c(1, 2, 3); invisible(tracemem(a$x))
a$x[[1]] <- a$x[[2]] * 3
print(a$x)
sink(); close(tc)
cat(norm(lines), sep = "\n")

lines <- character(0)
tc <- textConnection("lines", "wr")
sink(tc)
b <- list(); b$x <- c(1, 2, 3); invisible(tracemem(b$x))
b$y <- "dummy"; b$x[[1]] <- 7
print(b$x)
sink(); close(tc)
cat(norm(lines), sep = "\n")

lines <- character(0)
tc <- textConnection("lines", "wr")
sink(tc)
c0 <- list(); c0$x <- c(NA); length(c0$x) <- 10; c0$x[1] <- 42
invisible(tracemem(c0$x)); c0$x[2:9] <- 42
print(c0$x[2])
sink(); close(tc)
cat(norm(lines), sep = "\n")

lines <- character(0)
tc <- textConnection("lines", "wr")
sink(tc)
d <- list(); d$x <- c(1, 2, 3); invisible(tracemem(d$x))
d$x[1:2] <- c(8, 9)
print(d$x)
sink(); close(tc)
cat(norm(lines), sep = "\n")

# aliases of the container are not modified
e <- list(x = c(1, 2, 3))
f <- e
e$x[[1]] <- 99
print(f$x)
print(e$x)

# deeper chains and name-keyed [[
g <- list(); g$u$v <- c(1, 2, 3)
g$u$v[[2]] <- 9
print(g$u$v)
h <- list(); h$x <- c(1, 2, 3); names(h$x) <- c("a", "b", "c")
h$x[["b"]] <- 7
print(h$x)

# a classed element still dispatches its replacement method
k <- list(); k$x <- c(1, 2, 3); class(k$x) <- "foo"
`[[<-.foo` <- function(x, i, value) { x[i] <- value * 100; x }
k$x[[1]] <- 2
print(k$x)

# errors leave the binding in place
m <- list(x = c(1, 2, 3))
r <- tryCatch(m$x[[1]] <- stop("boom"), error = conditionMessage)
cat(r, "\n")
print(m$x)
