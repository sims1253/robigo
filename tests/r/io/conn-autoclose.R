# gc() closes the connections nothing refers to anymore (connections.c's
# conFinalizer), warning for those that were open; a sink keeps its own.
setwd(tempdir())

writer <- function() {
    con <- file("autoclose-a.txt", "w")
    writeLines("written", con)
    invisible(NULL)
}
writer()
invisible(gc())
cat("after gc 1\n")
print(readLines("autoclose-a.txt"))

# never opened: closed without a warning
unopened <- function() {
    file("autoclose-b.txt")
    invisible(NULL)
}
unopened()
invisible(gc())
cat("after gc 2\n")
print(nrow(showConnections()))

# a text connection never warns
texts <- function() {
    tc <- textConnection("abc")
    invisible(NULL)
}
texts()
invisible(gc())
cat("after gc 3\n")

# kept while something refers to it
keep <- file("autoclose-c.txt", "w")
invisible(gc())
print(isOpen(keep))
close(keep)

# a sink's connection stays open while it is a sink
local(sink(file("autoclose-d.txt", "w")))
invisible(NULL)
invisible(gc())
cat("into the sink\n")
sink()
invisible(gc())
cat("after gc 4\n")
print(readLines("autoclose-d.txt"))

# several at once, warned newest first
for (nm in c("autoclose-e.txt", "autoclose-f.txt")) {
    local(open(file(nm, "w")))
}
invisible(NULL)
invisible(gc())
cat("after gc 5\n")

# NextConnection reclaims unused ones when the table is full
for (k in 1:200) file("autoclose-g.txt")
cat("made 200\n")
print(nrow(showConnections(all = TRUE)) <= 128)
invisible(gc())
print(nrow(showConnections(all = TRUE)))
