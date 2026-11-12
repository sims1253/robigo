# browser(skipCalls =) walks R's `skipCalls--` test: NA or a negative
# count never reaches 0 and lands at top level.
k <- function(n) { browser(skipCalls = n); 4 }
m <- function(n) k(n)
m(-1L)
c
m(NA)
c
m(1L)
c
cat("end\n")
