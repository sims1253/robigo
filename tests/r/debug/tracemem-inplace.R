# tracemem over complex assignments and growing replacements. GNU R
# modifies a list element in place through `l$x[i] <- v` when nothing else
# holds the list (no report), and duplicates a shared vector before it
# grows it (subassign.c:EnlargeVector, a report). Every copy runs at the
# top level (an empty byte-code trace); addresses are normalized.
norm <- function(lines) gsub("0x[0-9a-f]+", "0xADDR", lines)

lines <- character(0)
tc <- textConnection("lines", "wr")
sink(tc)
l <- list(x = c(1, 2, 3))
invisible(tracemem(l$x))
l$x[2] <- 5
sink()
close(tc)
cat("l$x[2] <- 5", ":", length(lines), "lines\n")
if (length(lines)) cat(norm(lines), sep = "\n")

lines <- character(0)
tc <- textConnection("lines", "wr")
sink(tc)
l <- list(x = c(1, 2, 3))
invisible(tracemem(l$x))
l$x[[2]] <- 5
sink()
close(tc)
cat("l$x[[2]] <- 5", ":", length(lines), "lines\n")
if (length(lines)) cat(norm(lines), sep = "\n")

lines <- character(0)
tc <- textConnection("lines", "wr")
sink(tc)
l <- list(x = c(1, 2, 3))
invisible(tracemem(l[["x"]]))
l[["x"]][2] <- 5
sink()
close(tc)
cat('l[["x"]][2] <- 5', ":", length(lines), "lines\n")
if (length(lines)) cat(norm(lines), sep = "\n")

lines <- character(0)
tc <- textConnection("lines", "wr")
sink(tc)
l <- list(x = c(1, 2, 3))
l$x[1] <- 0
invisible(tracemem(l$x))
l$x[2] <- 5
sink()
close(tc)
cat("after l$x[1] <- 0", ":", length(lines), "lines\n")
if (length(lines)) cat(norm(lines), sep = "\n")

lines <- character(0)
tc <- textConnection("lines", "wr")
sink(tc)
l <- list(a = list(x = c(1, 2, 3)))
invisible(tracemem(l$a$x))
l$a$x[2] <- 5
sink()
close(tc)
cat("nested", ":", length(lines), "lines\n")
if (length(lines)) cat(norm(lines), sep = "\n")

lines <- character(0)
tc <- textConnection("lines", "wr")
sink(tc)
l <- list()
l$x <- c(NA)
length(l$x) <- 10
l$x[1] <- 42
invisible(tracemem(l$x))
l$x[2:9] <- 42
sink()
close(tc)
cat("fastr TestSimpleLists", ":", length(lines), "lines\n")
if (length(lines)) cat(norm(lines), sep = "\n")

lines <- character(0)
tc <- textConnection("lines", "wr")
sink(tc)
l <- list(x = c(1, 2, 3))
m <- l
invisible(tracemem(l$x))
l$x[2] <- 5
sink()
close(tc)
cat("shared list", ":", length(lines), "lines\n")
if (length(lines)) cat(norm(lines), sep = "\n")

lines <- character(0)
tc <- textConnection("lines", "wr")
sink(tc)
x <- c(1, 2)
invisible(tracemem(x))
y <- x
x[3] <- 1
sink()
close(tc)
cat("grow shared vector", ":", length(lines), "lines\n")
if (length(lines)) cat(norm(lines), sep = "\n")

lines <- character(0)
tc <- textConnection("lines", "wr")
sink(tc)
x <- list(1, 2)
invisible(tracemem(x))
y <- x
x[[3]] <- 1
sink()
close(tc)
cat("grow shared list", ":", length(lines), "lines\n")
if (length(lines)) cat(norm(lines), sep = "\n")

lines <- character(0)
tc <- textConnection("lines", "wr")
sink(tc)
x <- c(1, 2)
invisible(tracemem(x))
x[3] <- 1
sink()
close(tc)
cat("grow unshared vector", ":", length(lines), "lines\n")
if (length(lines)) cat(norm(lines), sep = "\n")

lines <- character(0)
tc <- textConnection("lines", "wr")
sink(tc)
x <- c(1, 2)
invisible(tracemem(x))
y <- x
y[1] <- 3
z <- y
z[1] <- 4
sink()
close(tc)
cat("copies stay traced", ":", length(lines), "lines\n")
if (length(lines)) cat(norm(lines), sep = "\n")

# A names vector that a name lookup indexed, held by one variable once its
# vector is gone: R still counts the attribute's reference and duplicates.
lines <- character(0)
tc <- textConnection("lines", "wr")
sink(tc)
x <- 1:40 + 0L
names(x) <- paste0("n", 1:40)
for (k in c("n5", "n6", "n7")) x[[k]]
nm <- names(x)
x <- 0
invisible(tracemem(nm))
nm[1] <- "q"
sink()
close(tc)
cat("name-indexed names", ":", length(lines), "lines\n")
if (length(lines)) cat(norm(lines), sep = "\n")

lines <- character(0)
tc <- textConnection("lines", "wr")
sink(tc)
x <- seq_len(40)
invisible(tracemem(x))
x[2] <- 0L
sink()
close(tc)
cat("compact sequence", ":", length(lines), "lines\n")
if (length(lines)) cat(norm(lines), sep = "\n")
