# R: debug.c / memory.c:memtrace_report — tracemem reports a copy of a
# traced object when a replacement ([[ <- , [ <- ) duplicates it. Plain
# assignment never copies, so it never reports. The context after "': '" is
# the byte-code trace buffer: empty when the copy happens in a top-level
# statement (the shape the fastr corpus records), so all copies here run at
# the top level inside sink() regions; addresses are normalized away.
norm <- function(lines) gsub("0x[0-9a-f]+", "0xADDR", lines)
lines <- character(0)

# two copies: x[[1]] <- 42 and y[[2]] <- 84 both duplicate the shared
# traced vector; x <- v and y <- v do not.
tc <- textConnection("lines", "wr")
sink(tc)
print(tracemem(v <- c(1, 10, 100)))
x <- v
y <- v
x[[1]] <- 42
y[[2]] <- 84
sink()
close(tc)
cat(length(lines), "lines\n")
cat(norm(lines), sep = "\n")

# untracemem stops the reports, also for the binding still shared with it.
lines <- character(0)
tc <- textConnection("lines", "wr")
sink(tc)
print(tracemem(v2 <- c(1, 10, 100)))
x2 <- v2
y2 <- v2
x2[[1]] <- 42
untracemem(v2)
y2[[2]] <- 84
sink()
close(tc)
cat(length(lines), "lines\n")
cat(norm(lines), sep = "\n")

# lists take the same path.
lines <- character(0)
tc <- textConnection("lines", "wr")
sink(tc)
print(tracemem(l <- list(1, 10, 100)))
x3 <- l
x3[[1]] <- 42
sink()
close(tc)
cat(length(lines), "lines\n")
cat(norm(lines), sep = "\n")

# retracemem reports "[<previous> -> new]" and traces its object; the copy
# that follows reports from the newly traced address. Subsetting created a
# fresh, untraced vector, so x4 <- v4[-1] reports nothing.
lines <- character(0)
tc <- textConnection("lines", "wr")
sink(tc)
print(tracemem(v4 <- c(1, 10, 100)))
x4 <- v4[-1]
retracemem(x4, retracemem(v4))
u4 <- x4
u4[[1]] <- 42
sink()
close(tc)
cat(length(lines), "lines\n")
cat(norm(lines), sep = "\n")

# [<-'s copy reports too.
lines <- character(0)
tc <- textConnection("lines", "wr")
sink(tc)
print(tracemem(v5 <- c(1, 10, 100)))
x5 <- v5
x5[c(1, 3)] <- c(7, 9)
sink()
close(tc)
cat(length(lines), "lines\n")
cat(norm(lines), sep = "\n")

# no modification: no report at all.
lines <- character(0)
tc <- textConnection("lines", "wr")
sink(tc)
print(tracemem(v6 <- c(1, 10, 100)))
x6 <- v6
y6 <- x6
print(x6)
sink()
close(tc)
cat(length(lines), "lines\n")
cat(norm(lines), sep = "\n")

# errors
cat(tryCatch(tracemem(function(x) x), error = conditionMessage), "\n")
cat(tryCatch(tracemem(NULL), error = conditionMessage), "\n")
cat(tryCatch(tracemem(new.env()), error = conditionMessage), "\n")
cat(tryCatch(retracemem(1:3, previous = 42), error = conditionMessage), "\n")
