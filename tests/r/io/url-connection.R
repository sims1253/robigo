# Gap: url() connections. Creating one (no I/O) is enough; jsonlite's
# Github tests need readLines(url(...)).
u <- url("https://cran.r-project.org/")
print(inherits(u, "connection"))
print(summary(u)$class)
print(isOpen(u))
close(u)
