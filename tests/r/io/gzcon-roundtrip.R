# gzcon() over file connections (connections.c:do_gzcon and the gzcon
# class): the issue's repro (gzfile writes, gzcon reads), the reverse
# (gzcon writes, gzfile reads), binary data, allowNonCompressed on a plain
# file, saveRDS/readRDS and save/load through gzcon(file(...)).
p <- tempfile(fileext = ".gz")
z <- gzfile(p, "w"); writeLines(c("a", "b"), z); close(z)
con <- gzcon(file(p, "rb"))
print(readLines(con))
close(con)

# written through gzcon, read through gzfile and as raw bytes
z <- gzcon(file(p, "wb"), level = 9)
print(class(z))
print(summary(z)[c("class", "mode", "text", "opened", "can read", "can write")])
writeLines(sprintf("line %d", 1:5), z)
writeBin(1:3, z)
close(z)
g <- gzfile(p, "rb")
print(readLines(g, n = 5))
print(readBin(g, "integer", n = 3))
close(g)
print(readBin(p, "raw", 4))

# a large body: deflate crosses many buffers both ways
x <- as.raw(sample(0:255, 200000, replace = TRUE))
z <- gzcon(file(p, "wb"))
writeBin(x, z)
close(z)
z <- gzcon(file(p, "rb"))
y <- readBin(z, "raw", 300000)
close(z)
print(identical(x, y))
z <- gzcon(file(p, "rb"))
y <- c(readBin(z, "raw", 70000), readBin(z, "raw", 70000), readBin(z, "raw", 70000))
print(identical(x, y))
print(length(readBin(z, "raw", 10)))
close(z)

# reading a line at a time, then the rest
z <- gzcon(file(p, "wb"))
writeLines(c("first", "second", "third"), z)
close(z)
z <- gzcon(file(p, "rb"))
print(isOpen(z))
print(readLines(z, n = 1))
print(readLines(z))
close(z)

# allowNonCompressed on a plain file
q <- tempfile()
writeLines(c("plain", "text"), q)
z <- gzcon(file(q, "rb"))
print(readLines(z))
close(z)
z <- gzcon(file(q, "rb"), allowNonCompressed = TRUE)
print(readChar(z, 3))
print(readLines(z))
close(z)
# (the wrapped file is open, so gzcon() opens the gzcon at once: the
# refusal is a warning there, and the gzcon stays open with nothing to read)
withCallingHandlers({
  z <- gzcon(file(q, "rb"), allowNonCompressed = FALSE)
  print(isOpen(z))
  print(tryCatch(readLines(z), error = conditionMessage))
  print(readBin(z, "raw", 10))
  close(z)
}, warning = function(w) { print(conditionMessage(w)); invokeRestart("muffleWarning") })
# opened later by open(): the refusal fails open(), yet the gzcon is open
withCallingHandlers({
  z <- gzcon(file(q), allowNonCompressed = FALSE)
  print(isOpen(z))
  print(tryCatch(open(z, "rb"), error = conditionMessage))
  print(isOpen(z))
  print(readLines(z))
  close(z)
}, warning = function(w) { print(conditionMessage(w)); invokeRestart("muffleWarning") })

# saveRDS / readRDS through gzcon(file(...))
obj <- list(a = 1:10, b = letters, c = list(d = pi))
z <- gzcon(file(p, "wb"))
saveRDS(obj, z)
close(z)
print(identical(readRDS(p), obj))
z <- gzcon(file(p, "rb"))
print(identical(readRDS(z), obj))
close(z)

# load() wraps a non-gzfile connection in gzcon (base/R/load.R)
aa <- 1:3; bb <- "x"
save(aa, bb, file = p)
rm(aa, bb)
print(tryCatch(load(file(p)), error = conditionMessage))
print(load(file(p, "rb")))
print(list(aa, bb))

# file() on gzip data in text mode is a gzfile connection (only the
# class attribute says "file"), so gzcon() does not warn about it below
f <- file(p, "r")
print(class(f))
print(summary(f)[c("class", "mode")])
print(class(getConnection(f)))
close(f)
f <- file(p)
print(summary(f)[c("class", "mode", "opened")])
close(f)
f <- file(p, "rb")
print(summary(f)$class)
close(f)

# argument checks and the text-mode file warning
print(tryCatch(gzcon(file(p, "rb"), level = 10), error = conditionMessage))
print(tryCatch(gzcon(file(p, "rb"), allowNonCompressed = NA), error = conditionMessage))
print(tryCatch(gzcon(file(p, "rb"), text = NA), error = conditionMessage))
print(tryCatch(gzcon(1), error = conditionMessage))
print(tryCatch(gzcon(file(p, "a")), error = conditionMessage))
withCallingHandlers({
  z <- gzcon(file(p, "r"))
  print(readLines(z, n = 1))
  close(z)
}, warning = function(w) { print(conditionMessage(w)); invokeRestart("muffleWarning") })
print(tryCatch(seek(z <- gzcon(file(p, "rb"))), error = conditionMessage))
close(z)
unlink(c(p, q))
