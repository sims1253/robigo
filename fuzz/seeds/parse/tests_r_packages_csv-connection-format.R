library(utils)
# Literal %s native writes, the stack-copy boundary, and %s%s row names.
x <- data.frame(number = c(0, 1.5, NA, Inf, NaN, -0),
                text = c("", strrep("a", 255), strrep("b", 256),
                         strrep("c", 257), "comma,\"quote\"\nnext", "café"),
                flag = c(TRUE, FALSE, NA, TRUE, FALSE, TRUE))
f <- tempfile()
for (rows in c(FALSE, TRUE)) {
    write.csv(x, f, row.names = rows)
    cat(readLines(f), sep = "\n")
    back <- read.csv(f, row.names = if (rows) 1 else NULL)
    print(back$number)
    print(identical(back$text, x$text))
    print(identical(back$flag, x$flag))
}
write.table(c(x$text[1:4], "percent %s%n"), f, quote = FALSE, row.names = FALSE, col.names = FALSE)
cat(readLines(f), sep = "\n")
write.table(x, f, sep = ";", dec = ",", eol = "\r\n", na = "missing",
            qmethod = "escape", row.names = FALSE)
cat(readLines(f), sep = "\n")
print(sum(readBin(f, "raw", 10000L) == as.raw(13L)))
unlink(f)
