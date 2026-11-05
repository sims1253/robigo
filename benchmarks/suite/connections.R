# Connections, byte by byte: readLines of a file, a text connection and a
# raw connection; readBin and writeLines; scan of a file.
n <- 100000
lines <- paste0("line ", 1:n, ": ", strrep("x", (1:n) %% 40))
f <- tempfile()
writeLines(lines, f)
back <- readLines(f)
con <- file(f, "r")
first <- readLines(con, 1000)
rest <- readLines(con)
close(con)
tcon <- textConnection(lines[1:20000])
tc <- readLines(tcon)
close(tcon)
rc <- rawConnection(charToRaw(paste(lines[1:20000], collapse = "\n")))
rl <- readLines(rc)
close(rc)
bytes <- readBin(f, raw(), file.info(f)$size)
sc <- scan(f, what = "", sep = "\n", quiet = TRUE)
cat(length(back), length(first) + length(rest), length(tc), length(rl), length(bytes), length(sc),
    sum(nchar(back)), "\n")
unlink(f)
